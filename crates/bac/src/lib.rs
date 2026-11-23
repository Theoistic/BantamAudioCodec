//! BAC, the Bantam Audio Codec: a voice codec and a labelled audio format.
//!
//! * `codec`, `encoder`: the audio bitstream. Linear prediction, pitch prediction and a range coder; one bitstream
//!   from a few kbit/s to lossless.
//! * `range`, `lpc`: its entropy coder and its predictor.
//! * `container`: the stream of units around it, made to be played while it arrives.
//! * `labels`: what the stream says about the audio: speakers, transcript, alignments, range tags.
//! * `pcm`, `flac`: plain samples, WAV and FLAC files in.
//! * `metrics`: how far a decoded signal is from its source.
//!
//! No dependencies. The format is written down in docs/Format.md.
//!
//! ```no_run
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let pcm = bac::pcm::read_wav(&std::fs::read("talk.wav")?)?;
//! let opts = bac::EncOpts { quality: 4.0, ..Default::default() }; // or lossless: true
//! let file = bac::encode(&pcm, &opts, &bac::Labels::default())?;
//! let (head, labels, back) = bac::decode(&file)?;
//! # let _ = (head, labels, back);
//! # Ok(())
//! # }
//! ```

// signal code reads best with the sample index in sight
#![allow(clippy::needless_range_loop)]

pub mod codec;
pub mod container;
pub mod encoder;
pub mod error;
pub mod flac;
pub mod json;
pub mod labels;
pub mod lpc;
pub mod metrics;
pub mod pcm;
pub mod range;
pub mod wire;

pub use codec::{Decoder, Params};
pub use container::{decode, decode_range, encode, read_index, rewrite_labels, scan, Head, Reader, Writer};
pub use encoder::{EncOpts, Encoder};
pub use error::{Error, Result};
pub use labels::{Attr, Event, Labels, Speaker, Value};
pub use pcm::Pcm;

#[cfg(test)]
mod tests {
    use super::*;

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 32) as u32
        }
        /// roughly normal, unit variance
        fn gauss(&mut self) -> f64 {
            (0..12).map(|_| self.next() as f64 / 4294967296.0).sum::<f64>() - 6.0
        }
    }

    /// Something like a voice: a pulse train with a gliding pitch through two resonances, pauses, and breath noise.
    fn voice(rate: u32, secs: f64, amp: f64, seed: u64) -> Vec<i32> {
        let mut rng = Rng(seed);
        let n = (rate as f64 * secs) as usize;
        let mut out = Vec::with_capacity(n);
        let (mut phase, mut y1, mut y2, mut z1, mut z2) = (0.0f64, 0.0, 0.0, 0.0, 0.0);
        for i in 0..n {
            let t = i as f64 / rate as f64;
            let f0 = 110.0 + 40.0 * (t * 2.1).sin();
            phase += f0 / rate as f64;
            let pulse = if phase >= 1.0 {
                phase -= 1.0;
                1.0
            } else {
                0.0
            };
            let speaking = (t * 1.3).sin() > -0.3;
            let voiced = (t * 3.7).sin() > -0.5;
            let src = if !speaking { 0.002 * rng.gauss() } else if voiced { pulse + 0.01 * rng.gauss() } else { 0.08 * rng.gauss() };
            // resonances at 700 Hz and 1800 Hz
            let res = |f: f64, bw: f64| {
                let r = (-std::f64::consts::PI * bw / rate as f64).exp();
                (2.0 * r * (2.0 * std::f64::consts::PI * f / rate as f64).cos(), -r * r)
            };
            let (a1, a2) = res(700.0, 110.0);
            let (b1, b2) = res(1800.0, 160.0);
            let y = src + a1 * y1 + a2 * y2;
            (y2, y1) = (y1, y);
            let z = y + b1 * z1 + b2 * z2;
            (z2, z1) = (z1, z);
            out.push((z * amp) as i32);
        }
        out
    }

    fn round_trip(x: &[Vec<i32>], p: Params, o: &EncOpts) -> (Vec<Vec<i32>>, usize) {
        let n = p.frame_len as usize;
        let frames = x[0].len().div_ceil(n);
        let mut enc = Encoder::new(p.clone(), o);
        let mut dec = Decoder::new(p).unwrap();
        let mut out = vec![Vec::new(); x.len()];
        let mut bytes = 0;
        let per = 7;
        for (pi, f0) in (0..frames).step_by(per).enumerate() {
            let nf = per.min(frames - f0);
            let key = pi % 3 == 0;
            let data = enc.encode_packet(x, f0, nf, key);
            bytes += data.len();
            if key {
                dec.reset();
            }
            dec.decode_packet(&data, nf, &mut out).unwrap();
        }
        for o in &mut out {
            o.truncate(x[0].len());
        }
        (out, bytes)
    }

    fn snr(a: &[i32], b: &[i32]) -> f64 {
        let (mut s, mut e) = (0.0, 0.0);
        for (&x, &y) in a.iter().zip(b) {
            s += (x as f64).powi(2);
            e += ((x - y) as f64).powi(2);
        }
        10.0 * (s / e.max(1e-9)).log10()
    }

    #[test]
    fn lossless_gives_the_input_back() {
        for (rate, bits, amp) in [(16_000u32, 16u8, 900.0), (8_000, 16, 2000.0), (22_050, 16, 600.0), (48_000, 24, 20_000.0)] {
            let p = Params::new(rate, 1, bits, true);
            let (lo, hi) = p.sample_range();
            let x = vec![voice(rate, 2.3, amp, 3 + rate as u64).into_iter().map(|v| (v as i64).clamp(lo, hi) as i32).collect::<Vec<_>>()];
            println!("{rate} Hz: peak {}", x[0].iter().map(|v| v.abs()).max().unwrap());
            let (y, bytes) = round_trip(&x, p, &EncOpts { lossless: true, ..Default::default() });
            assert!(x == y, "{rate} Hz {bits} bits");
            let raw = x[0].len() * bits as usize / 8;
            assert!(bytes * 10 < raw * 7, "{rate} Hz: {bytes} of {raw} bytes");
        }
    }

    #[test]
    fn lossless_survives_noise_silence_and_full_scale() {
        let mut rng = Rng(99);
        let mut x: Vec<i32> = (0..5000).map(|_| (rng.next() >> 16) as i32 - 32768).collect();
        x.extend(std::iter::repeat_n(0, 1000));
        x.extend((0..3000).map(|i| if i % 2 == 0 { 32767 } else { -32768 }));
        x.extend((0..777).map(|i| (i % 50) - 25));
        let x = vec![x];
        let (y, _) = round_trip(&x, Params::new(16_000, 1, 16, true), &EncOpts { lossless: true, ..Default::default() });
        assert!(x == y);
    }

    #[test]
    fn two_channels_are_kept_apart() {
        let x = vec![voice(16_000, 1.1, 900.0, 5), voice(16_000, 1.1, 400.0, 6)];
        let (y, _) = round_trip(&x, Params::new(16_000, 2, 16, true), &EncOpts { lossless: true, ..Default::default() });
        assert!(x == y);
    }

    #[test]
    fn one_voice_on_two_channels_costs_one() {
        let v = voice(16_000, 2.0, 900.0, 7);
        // the second channel is the first, but for a stretch in the middle where it has a voice of its own
        let mut other = v.clone();
        other[12_000..20_000].copy_from_slice(&voice(16_000, 0.5, 500.0, 8));
        let x = vec![v.clone(), other];
        for lossless in [true, false] {
            let o = EncOpts { lossless, ..Default::default() };
            let (one, mono) = round_trip(std::slice::from_ref(&v), Params::new(16_000, 1, 16, lossless), &o);
            let (two, twin) = round_trip(&[v.clone(), v.clone()], Params::new(16_000, 2, 16, lossless), &o);
            assert!(two[0] == one[0] && two[1] == one[0]);
            assert!(twin < mono + mono / 50, "{twin} bytes for two, {mono} for one");
            let (y, mixed) = round_trip(&x, Params::new(16_000, 2, 16, lossless), &o);
            // frame 37 (samples 11,840 to 12,160) is the first that differs
            assert!(mixed > twin && y[1][..11_840] == y[0][..11_840]);
            if lossless {
                assert!(x == y);
            }
        }
    }

    #[test]
    fn lossy_is_smaller_and_close() {
        let x = vec![voice(16_000, 4.0, 900.0, 11)];
        let p = Params::new(16_000, 1, 16, false);
        let (_, lossless) = round_trip(&x, Params::new(16_000, 1, 16, true), &EncOpts { lossless: true, ..Default::default() });
        let mut last = (0usize, f64::MAX);
        for q in [8.0f32, 5.0, 2.0] {
            let (y, bytes) = round_trip(&x, p.clone(), &EncOpts { quality: q, ..Default::default() });
            let s = snr(&x[0], &y[0]);
            println!("q {q}: {bytes} bytes ({:.1} kbit/s), SNR {s:.1} dB; lossless {lossless}", bytes as f64 * 8.0 / 4000.0);
            assert!(bytes < lossless && bytes > last.0 / 8 && s < last.1 + 0.5, "q {q}");
            assert!(s > 3.0, "q {q}: SNR {s:.1} dB");
            last = (bytes, s);
        }
    }

    #[test]
    fn the_same_packet_decodes_the_same_everywhere() {
        // two decoders, one fed packet by packet from the start, one joining at a key packet: the same samples
        let x = vec![voice(16_000, 3.0, 900.0, 21)];
        let p = Params::new(16_000, 1, 16, false);
        let n = p.frame_len as usize;
        let mut enc = Encoder::new(p.clone(), &EncOpts::default());
        let packets: Vec<Vec<u8>> = (0..10).map(|i| enc.encode_packet(&x, i * 10, 10, i % 5 == 0)).collect();
        let mut a = Decoder::new(p.clone()).unwrap();
        let mut full = vec![Vec::new()];
        for (i, d) in packets.iter().enumerate() {
            if i % 5 == 0 {
                a.reset();
            }
            a.decode_packet(d, 10, &mut full).unwrap();
        }
        let mut b = Decoder::new(p).unwrap();
        let mut tail = vec![Vec::new()];
        for d in &packets[5..] {
            b.decode_packet(d, 10, &mut tail).unwrap();
        }
        assert!(full[0][50 * n..] == tail[0][..]);
    }

    #[test]
    fn damaged_packets_do_not_panic() {
        let x = vec![voice(16_000, 1.0, 900.0, 31)];
        let p = Params::new(16_000, 1, 16, false);
        let mut enc = Encoder::new(p.clone(), &EncOpts::default());
        let good = enc.encode_packet(&x, 0, 20, true);
        let mut rng = Rng(5);
        for _ in 0..300 {
            let mut bad = good.clone();
            for _ in 0..1 + rng.next() % 6 {
                let i = (rng.next() as usize) % bad.len();
                bad[i] ^= 1 << (rng.next() % 8);
            }
            bad.truncate(1 + (rng.next() as usize) % bad.len());
            let mut dec = Decoder::new(p.clone()).unwrap();
            let _ = dec.decode_packet(&bad, 20, &mut [Vec::new()]);
        }
    }
}
