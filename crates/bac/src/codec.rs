//! The audio bitstream and its decoder.
//!
//! A frame (20 ms) of one channel is rebuilt sample by sample, in integers:
//!
//! ```text
//! excitation = step * u  (or a small noise where u = 0)  +  long-term prediction (pitch: the excitation one lag back)
//! output     = excitation + short-term prediction (the last `order` output samples under the frame's LPC filter)
//! ```
//!
//! `u` are the coded integers. With step 1 everywhere the output is the input, bit for bit: lossless is the same
//! bitstream as lossy, with the finest step. Everything a perceptual model decides (the steps, where the noise goes)
//! is the encoder's business and is not part of the format.
//!
//! Frame syntax, every item under an adaptive table of the range coder unless said otherwise:
//!
//! ```text
//! type                    0 silent (all samples 0), 1 unvoiced, 2 voiced, 3 copy (the frame is the same frame of the
//!                         channel before: two channels with one voice cost one); table chosen by the previous type
//! -- types 1, 2:
//! repeat                  1 = keep the previous frame's predictor
//! order x index delta     (if not repeat) reflection grid indices, each minus the previous frame's, modulo its alphabet
//! fill                    0..3: the noise put where u = 0 is step * (0, 1/8, 3/16, 1/4), sign from a fixed generator
//! 4 x subframe:
//!     step delta          step index minus the previous subframe's, modulo 96; step = 2^(index / 4)
//!     -- type 2:
//!     lag                 first subframe: plain bits, lag - lag_min; later ones: delta to the previous lag + 8, 0..15
//!     gain                0 = no long-term prediction here, 1..7 = 0.2 .. 1.1
//!     shape               (if gain > 0) 0 = taps (1/8, 3/4, 1/8), 1 = taps (1/4, 1/2, 1/4), times the gain
//!     samples x u         token |u| >> k (15 = escape, then 5 bits of length and the value in plain bits), k plain low
//!                         bits, a sign bit if u != 0. k and the token's table follow the last magnitudes.
//! ```

use crate::error::{bad, Error, Result};
use crate::lpc;
use crate::range::{Cdf, RangeDecoder};

pub const SUBFRAMES: usize = 4;
pub const MAX_ORDER: usize = 32;
/// predictor coefficients are Q14
pub const COEF_SHIFT: u32 = 14;
pub const STEP_SYMS: usize = 96;
pub const LAG_DELTAS: usize = 16;
pub const LAG_DELTA_BIAS: i32 = 8;
pub const LTP_GAINS: usize = 8;
pub const FILL_LEVELS: usize = 4;
pub const TOK_SYMS: usize = 16;
pub const TOK_ESC: u32 = 15;
pub const FT_SILENT: usize = 0;
pub const FT_UNVOICED: usize = 1;
pub const FT_VOICED: usize = 2;
pub const FT_COPY: usize = 3;
pub const FRAME_TYPES: usize = 4;
/// tables for the token: 8 by the running mean magnitude, 4 by the last two magnitudes
const TOK_CTX: usize = 32;
/// where the noise generator starts after a reset
const SEED: u32 = 0x2545_F491;
/// frames per packet at most: bounds what a damaged count can ask for
pub const MAX_PACKET_FRAMES: usize = 4096;

/// The quantiser step of index `idx` in Q4: 2^(idx / 4), from 1 (index 0) up.
#[inline]
pub fn step_q4(idx: u8) -> i64 {
    const BASE: [i64; 4] = [16, 19, 23, 27];
    BASE[(idx & 3) as usize] << (idx >> 2)
}

/// The three long-term taps in Q7 for the samples lag + 1, lag and lag - 1 back.
pub fn ltp_taps(gain: usize, shape: usize) -> [i32; 3] {
    const GAIN_Q7: [i32; LTP_GAINS] = [0, 26, 45, 64, 83, 102, 122, 141];
    let g = GAIN_Q7[gain];
    let side = if shape == 0 { (g + 4) >> 3 } else { (g + 2) >> 2 };
    [side, g - 2 * side, side]
}

/// The size of the noise that stands in for u = 0 at this step and fill level.
#[inline]
pub fn fill_amp(step_q4: i64, level: usize) -> i32 {
    const FILL_Q8: [i64; FILL_LEVELS] = [0, 2, 3, 4];
    ((step_q4 * FILL_Q8[level] + 128) >> 8).min(i32::MAX as i64) as i32
}

/// What a decoder must know about a stream; the HEAD unit of the container carries it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Params {
    pub sample_rate: u32,
    pub channels: u8,
    /// bits per sample of the source, 8 to 24; output samples are clamped to this range
    pub bits: u8,
    /// the encoder's word that the output equals the source, sample for sample
    pub lossless: bool,
    /// samples per frame
    pub frame_len: u32,
    /// short-term predictor order
    pub order: u8,
    pub lag_min: u32,
    pub lag_max: u32,
}

impl Params {
    /// The encoder's choices for a sample rate: 20 ms frames, a predictor order that grows with the bandwidth, and
    /// pitch lags for 56 to 500 Hz.
    pub fn new(sample_rate: u32, channels: u8, bits: u8, lossless: bool) -> Params {
        let order = match sample_rate {
            0..=9_000 => 10,
            9_001..=13_000 => 12,
            13_001..=18_000 => 16,
            18_001..=26_000 => 20,
            _ => 24,
        };
        Params {
            sample_rate,
            channels,
            bits,
            lossless,
            frame_len: (sample_rate / 50).max(16),
            order,
            lag_min: (sample_rate / 500).max(2),
            lag_max: (sample_rate / 56).max(8),
        }
    }

    pub fn check(&self) -> Result<()> {
        let ok = (1..=768_000).contains(&self.sample_rate)
            && (1..=8).contains(&self.channels)
            && (8..=24).contains(&self.bits)
            && (16..=16_384).contains(&self.frame_len)
            && (1..=MAX_ORDER as u8).contains(&self.order)
            && self.lag_min >= 2
            && self.lag_min <= self.lag_max
            && self.lag_max <= 16_384;
        if ok {
            Ok(())
        } else {
            Err(Error::Unsupported(format!("stream parameters {self:?}")))
        }
    }

    /// Plain bits of a subframe's first lag.
    pub fn lag_bits(&self) -> u32 {
        32 - (self.lag_max - self.lag_min).leading_zeros()
    }

    /// Samples of the past that a channel keeps.
    pub fn hist(&self) -> usize {
        (self.lag_max as usize + 2).max(self.order as usize)
    }

    /// The sample range [from, to) of subframe `sf` inside a frame.
    pub fn sub(&self, sf: usize) -> (usize, usize) {
        let n = self.frame_len as usize;
        (sf * n / SUBFRAMES, (sf + 1) * n / SUBFRAMES)
    }

    /// The lowest and highest sample value.
    pub fn sample_range(&self) -> (i64, i64) {
        (-(1i64 << (self.bits - 1)), (1i64 << (self.bits - 1)) - 1)
    }
}

/// The adaptive tables of one channel.
pub struct Models {
    pub ftype: [Cdf; FRAME_TYPES],
    pub repeat: Cdf,
    pub lpc: Vec<Cdf>,
    /// [first subframe of a frame, later ones]
    pub step: [Cdf; 2],
    pub fill: Cdf,
    pub lag: Cdf,
    pub gain: Cdf,
    pub shape: Cdf,
    pub tok: Vec<Cdf>,
}

impl Models {
    fn new(order: usize) -> Models {
        let lag_w: Vec<u32> = (0..LAG_DELTAS as i32).map(|s| 1 + 32 / (1 + (s - LAG_DELTA_BIAS).unsigned_abs())).collect();
        Models {
            ftype: std::array::from_fn(|_| Cdf::uniform(FRAME_TYPES)),
            repeat: Cdf::uniform(2),
            lpc: (0..order).map(|i| Cdf::circular(lpc::alphabet(i))).collect(),
            step: [Cdf::circular(STEP_SYMS), Cdf::circular(STEP_SYMS)],
            fill: Cdf::uniform(FILL_LEVELS),
            lag: Cdf::from_weights(&lag_w),
            gain: Cdf::uniform(LTP_GAINS),
            shape: Cdf::uniform(2),
            tok: (0..TOK_CTX).map(|_| Cdf::falling(TOK_SYMS)).collect(),
        }
    }
}

/// Everything one channel carries from sample to sample. The encoder holds the same state and moves it with the
/// same functions, which is what keeps the two in step.
pub struct Chan {
    /// output samples: `hist` of the past, then the frame being built
    pub y: Vec<i32>,
    /// the excitation, laid out like `y`
    pub exc: Vec<i32>,
    pub hist: usize,
    /// the predictor in use: grid indices and the coefficients they stand for
    pub idx: [i16; MAX_ORDER],
    pub coef: [i32; MAX_ORDER],
    pub step: u8,
    pub prev_type: u8,
    seed: u32,
    /// running mean of |u| in Q5, over about 16 samples
    avg: u32,
    /// the last two |u|
    u1: u32,
    u2: u32,
    pub m: Models,
}

impl Chan {
    pub fn new(p: &Params) -> Chan {
        let (h, n) = (p.hist(), p.frame_len as usize);
        Chan {
            y: vec![0; h + n],
            exc: vec![0; h + n],
            hist: h,
            idx: [0; MAX_ORDER],
            coef: [0; MAX_ORDER],
            step: 0,
            prev_type: 0,
            seed: SEED,
            avg: 0,
            u1: 0,
            u2: 0,
            m: Models::new(p.order as usize),
        }
    }

    /// The short-term prediction of the sample at `pos`.
    #[inline]
    pub fn st(&self, pos: usize, order: usize) -> i64 {
        let w = &self.y[pos - order..pos];
        let mut s = 0i64;
        for j in 0..order {
            s += self.coef[j] as i64 * w[order - 1 - j] as i64;
        }
        (s + (1 << (COEF_SHIFT - 1))) >> COEF_SHIFT
    }

    /// The long-term prediction of the excitation at `pos`.
    #[inline]
    pub fn lt(&self, pos: usize, lag: usize, taps: &[i32; 3]) -> i64 {
        if taps[1] == 0 {
            return 0;
        }
        let b = pos - lag;
        (taps[0] as i64 * self.exc[b - 1] as i64 + taps[1] as i64 * self.exc[b] as i64 + taps[2] as i64 * self.exc[b + 1] as i64 + 64) >> 7
    }

    /// Builds the sample at `pos` from its predictions and the coded integer `u`.
    #[inline]
    #[allow(clippy::too_many_arguments)]
    pub fn commit(&mut self, pos: usize, st: i64, lt: i64, u: i32, step_q4: i64, fill: i32, lo: i64, hi: i64) -> i32 {
        let e = if u != 0 {
            let m = (step_q4 * u.unsigned_abs() as i64 + 8) >> 4;
            if u < 0 {
                -m
            } else {
                m
            }
        } else if fill != 0 {
            self.seed = self.seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            if self.seed >> 31 != 0 {
                -(fill as i64)
            } else {
                fill as i64
            }
        } else {
            0
        };
        let x = (e + lt).clamp(i32::MIN as i64, i32::MAX as i64);
        let y = (st + x).clamp(lo, hi) as i32;
        self.exc[pos] = x as i32;
        self.y[pos] = y;
        y
    }

    /// For the next |u|: how many low bits go plain, and which table codes the rest.
    #[inline]
    pub fn res_ctx(&self) -> (u32, usize) {
        let mean = self.avg >> 5;
        let (k, a) = if mean < 2 {
            (0, match self.avg {
                0..=3 => 0,
                4..=7 => 1,
                8..=15 => 2,
                16..=31 => 3,
                32..=47 => 4,
                _ => 5,
            })
        } else {
            let lg = 31 - mean.leading_zeros();
            (lg - 1, 6 + ((mean >> (lg - 1)) & 1) as usize)
        };
        let near = (self.u1.saturating_add(self.u2) >> k).min(3) as usize;
        (k, a * 4 + near)
    }

    #[inline]
    pub fn res_update(&mut self, mag: u32) {
        let m = mag.min(1 << 26);
        self.avg = (self.avg as i64 + ((((m as i64) << 5) - self.avg as i64) >> 4)) as u32;
        self.u2 = self.u1;
        self.u1 = m;
    }

    /// Moves the frame just built into the past.
    pub fn slide(&mut self, n: usize) {
        let h = self.hist;
        self.y.copy_within(n..n + h, 0);
        self.exc.copy_within(n..n + h, 0);
    }

    /// A frame of zeros.
    pub fn silent_frame(&mut self, n: usize) {
        let h = self.hist;
        self.y[h..h + n].fill(0);
        self.exc[h..h + n].fill(0);
        self.prev_type = FT_SILENT as u8;
    }

    /// The frame another channel has just built, taken over as this channel's.
    pub fn copy_frame(&mut self, from: &Chan, n: usize) {
        let h = self.hist;
        self.y[h..h + n].copy_from_slice(&from.y[h..h + n]);
        self.exc[h..h + n].copy_from_slice(&from.exc[h..h + n]);
        self.prev_type = FT_COPY as u8;
    }
}

/// Decodes packets of frames into samples.
pub struct Decoder {
    pub p: Params,
    ch: Vec<Chan>,
}

impl Decoder {
    pub fn new(p: Params) -> Result<Decoder> {
        p.check()?;
        let ch = (0..p.channels).map(|_| Chan::new(&p)).collect();
        Ok(Decoder { p, ch })
    }

    /// Back to the starting state: what a key packet asks for before its first frame.
    pub fn reset(&mut self) {
        for c in &mut self.ch {
            *c = Chan::new(&self.p);
        }
    }

    /// Decodes `nframes` frames of every channel and appends them to `out` (one buffer per channel).
    pub fn decode_packet(&mut self, data: &[u8], nframes: usize, out: &mut [Vec<i32>]) -> Result<()> {
        if nframes == 0 || nframes > MAX_PACKET_FRAMES {
            return Err(bad(format!("a packet of {nframes} frames")));
        }
        let mut rd = RangeDecoder::new(data);
        let n = self.p.frame_len as usize;
        for _ in 0..nframes {
            for c in 0..self.ch.len() {
                let (before, rest) = self.ch.split_at_mut(c);
                decode_frame(&self.p, &mut rest[0], before.last(), &mut rd)?;
            }
            for (ch, o) in self.ch.iter_mut().zip(out.iter_mut()) {
                o.extend_from_slice(&ch.y[ch.hist..ch.hist + n]);
                ch.slide(n);
            }
        }
        Ok(())
    }
}

/// `before`: the channel before this one, with its frame of the same time already built.
fn decode_frame(p: &Params, ch: &mut Chan, before: Option<&Chan>, rd: &mut RangeDecoder) -> Result<()> {
    let (n, order, h) = (p.frame_len as usize, p.order as usize, ch.hist);
    let ft = rd.decode(&mut ch.m.ftype[ch.prev_type as usize]);
    if ft == FT_SILENT {
        ch.silent_frame(n);
        return Ok(());
    }
    if ft == FT_COPY {
        ch.copy_frame(before.ok_or_else(|| bad("the first channel copies a channel before it"))?, n);
        return Ok(());
    }
    ch.prev_type = ft as u8;
    if rd.decode(&mut ch.m.repeat) == 0 {
        for i in 0..order {
            let a = lpc::alphabet(i) as i32;
            let sym = rd.decode(&mut ch.m.lpc[i]) as i32;
            let l1 = (a - 1) / 2;
            ch.idx[i] = ((ch.idx[i] as i32 + l1 + sym) % a - l1) as i16;
        }
        ch.coef = lpc::coefs_from_idx(&ch.idx, order);
    }
    let fill = rd.decode(&mut ch.m.fill);
    let (lo, hi) = p.sample_range();
    let mut lag = 0usize;
    for sf in 0..SUBFRAMES {
        let (a, b) = p.sub(sf);
        let sym = rd.decode(&mut ch.m.step[(sf > 0) as usize]);
        ch.step = ((ch.step as usize + sym) % STEP_SYMS) as u8;
        let sq = step_q4(ch.step);
        let fa = fill_amp(sq, fill);
        let mut taps = [0i32; 3];
        if ft == FT_VOICED {
            lag = if sf == 0 {
                p.lag_min as usize + rd.bits(p.lag_bits()) as usize
            } else {
                (lag as i64 + rd.decode(&mut ch.m.lag) as i64 - LAG_DELTA_BIAS as i64).max(0) as usize
            };
            if lag < p.lag_min as usize || lag > p.lag_max as usize {
                return Err(bad("a pitch lag outside the stream's range"));
            }
            let g = rd.decode(&mut ch.m.gain);
            if g > 0 {
                taps = ltp_taps(g, rd.decode(&mut ch.m.shape));
            }
        }
        for pos in h + a..h + b {
            let u = decode_residual(ch, rd);
            let st = ch.st(pos, order);
            let lt = ch.lt(pos, lag, &taps);
            ch.commit(pos, st, lt, u, sq, fa, lo, hi);
        }
    }
    Ok(())
}

#[inline]
fn decode_residual(ch: &mut Chan, rd: &mut RangeDecoder) -> i32 {
    let (k, ctx) = ch.res_ctx();
    let mut top = rd.decode(&mut ch.m.tok[ctx]) as u32;
    if top == TOK_ESC {
        let nb = rd.bits(5);
        top = (1u32 << nb).wrapping_add(rd.bits_long(nb)).wrapping_add(TOK_ESC - 1);
    }
    let mag = if k > 0 { (top.wrapping_shl(k) | rd.bits_long(k)).min(i32::MAX as u32) } else { top.min(i32::MAX as u32) };
    let neg = mag != 0 && rd.bits(1) != 0;
    ch.res_update(mag);
    if neg {
        -(mag as i32)
    } else {
        mag as i32
    }
}
