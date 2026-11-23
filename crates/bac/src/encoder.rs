//! The encoder. It holds the decoder's state (`Chan`) and moves it with the decoder's own functions, so what it
//! predicts from is exactly what a decoder will have. All of its judgement (the predictor, the pitch, how coarse a
//! step the ear allows, where the noise may go) can change without touching the format.
//!
//! Lossy coding is noise feedback coding, the structure of Opus's speech layer: the step of a subframe follows the
//! size of what prediction leaves over, and the rounding error of earlier samples is fed back through the predictor
//! with its poles pulled in, so that the noise sits under the formants where the voice masks it.

use crate::codec::*;
use crate::lpc;
use crate::range::RangeEncoder;

#[derive(Clone, Debug)]
pub struct EncOpts {
    /// 0 (smallest) to 10 (finest); not used when `lossless`
    pub quality: f32,
    pub lossless: bool,
    /// audio per packet: the unit a stream is cut into
    pub packet_ms: u32,
    /// a packet that can be decoded on its own at least this often: where a player can seek or join
    pub key_ms: u32,
    /// lossy only: code the background between words as shaped noise of the right colour and level (what phones
    /// call comfort noise) and not as a waveform. false keeps the recording's own noise, at several kbit/s more.
    pub model_noise: bool,
}

impl Default for EncOpts {
    fn default() -> Self {
        EncOpts { quality: 4.0, lossless: false, packet_ms: 500, key_ms: 1000, model_noise: true }
    }
}

/// Where an encoder's bits went, for tuning and for the curious. Bits are what the range coder was asked to spend
/// (the tables' own estimate), so the parts add up to the output within a fraction of a percent.
#[derive(Clone, Debug, Default)]
pub struct EncStats {
    /// frames by type: silent, unvoiced, voiced, copy of the channel before
    pub frames: [u64; FRAME_TYPES],
    /// frames coded as background noise
    pub background: u64,
    /// frames that kept the previous predictor
    pub repeats: u64,
    /// side information in bits: frame type and flags, predictor, steps, pitch lags and gains
    pub bits_flags: f64,
    pub bits_lpc: f64,
    pub bits_step: f64,
    pub bits_pitch: f64,
    /// all bytes of all packets
    pub bytes: u64,
}

/// What the quality number turns into.
struct Tuning {
    /// step over the RMS of the prediction residual, for a voiced subframe
    ratio: f64,
    /// no step below this
    floor: f64,
    /// how far the noise follows the spectrum: 0 = white noise, 1 = the shape of the voice
    gamma: f32,
    /// values this much short of a rounding boundary still round down: more zeros, which are cheap, for a little
    /// more error
    dead: f32,
    /// a frame is voiced above this normalised correlation at its pitch lag
    voiced: f64,
    /// background frames take steps this many times coarser, with noise in place of the zeros; 1 = no such thing
    background: f64,
    /// a subframe keeps the last step index when the one it wants is no further away than this
    hold: u8,
}

impl Tuning {
    fn new(o: &EncOpts, bits: u8) -> Tuning {
        let q = o.quality.clamp(0.0, 10.0) as f64;
        let ratio = 4.0 * 2f64.powf(-q / 2.5);
        let scale = (1u32 << (bits.max(16) - 16)) as f64;
        // coarse steps want the noise well under the formants and many zeros; fine steps are better off with
        // whiter noise and plain rounding. Both set from sweeps over LibriSpeech (docs/Research.md, "Tuning")
        let coarse = ratio.log2();
        Tuning {
            ratio,
            floor: (4.0 * ratio * scale).max(1.0),
            gamma: (0.7 + 0.15 * coarse).clamp(0.6, 0.85) as f32,
            dead: (0.25 + 0.08 * coarse).clamp(0.15, 0.35) as f32,
            voiced: 0.6,
            background: if o.model_noise && !o.lossless { 4.0 } else { 1.0 },
            hold: 1,
        }
    }
}

/// The encoder's own memory of a channel.
struct EncChan {
    /// output minus input of the last samples, the newest first
    err: [f32; MAX_ORDER],
    /// the RMS of every frame of the signal, taken once: what tells background from speech
    levels: Vec<f32>,
}

#[derive(Default)]
struct Scratch {
    /// input samples: the past a frame's analysis needs, the frame, and a little of what follows
    xb: Vec<i32>,
    /// what the frame's predictor leaves of the input, laid out like `Chan::exc`
    d: Vec<f32>,
    /// `d` below about 1 kHz, and a buffer on the way there
    low: Vec<f32>,
    low1: Vec<f32>,
    score: Vec<f64>,
    near: Vec<f32>,
}

/// dst[i] = the sum of the last `l` values of src up to i.
fn running_sum(src: &[f32], dst: &mut Vec<f32>, l: usize) {
    dst.clear();
    let mut acc = 0.0f32;
    for i in 0..src.len() {
        acc += src[i];
        if i >= l {
            acc -= src[i - l];
        }
        dst.push(acc);
    }
}

pub struct Encoder {
    pub p: Params,
    t: Tuning,
    an: lpc::Analysis,
    ch: Vec<Chan>,
    es: Vec<EncChan>,
    s: Scratch,
    pub stats: EncStats,
}

impl Encoder {
    pub fn new(p: Params, o: &EncOpts) -> Encoder {
        let n = p.frame_len as usize;
        let an = lpc::Analysis::new(n + 2 * (n / 4), p.sample_rate, p.lossless);
        let ch = (0..p.channels).map(|_| Chan::new(&p)).collect();
        let es = (0..p.channels).map(|_| EncChan { err: [0.0; MAX_ORDER], levels: Vec::new() }).collect();
        Encoder { t: Tuning::new(o, p.bits), an, ch, es, s: Scratch::default(), stats: EncStats::default(), p }
    }

    /// Codes frames `first_frame .. first_frame + nframes` of the signal `x` (one slice per channel; samples past
    /// its end count as zeros) into one packet. `key`: start from the reset state, so that the packet decodes on
    /// its own.
    pub fn encode_packet(&mut self, x: &[Vec<i32>], first_frame: usize, nframes: usize, key: bool) -> Vec<u8> {
        if key {
            for (c, e) in self.ch.iter_mut().zip(&mut self.es) {
                *c = Chan::new(&self.p);
                e.err = [0.0; MAX_ORDER];
            }
        }
        let n = self.p.frame_len as usize;
        for (e, x) in self.es.iter_mut().zip(x) {
            if e.levels.len() != x.len().div_ceil(n) {
                e.levels = x.chunks(n).map(|f| (f.iter().map(|&v| v as f64 * v as f64).sum::<f64>() / n as f64).sqrt() as f32).collect();
            }
        }
        let mut enc = RangeEncoder::new();
        for f in first_frame..first_frame + nframes {
            for c in 0..self.ch.len() {
                let (before, rest) = self.ch.split_at_mut(c);
                let ch = &mut rest[0];
                // the same samples as the channel before (one voice on two channels): take its frame over
                let frame = |x: &[i32]| x.get(f * n..).map_or(&[][..], |t| &t[..n.min(t.len())]).to_vec();
                if c > 0 && frame(&x[c]) == frame(&x[c - 1]) && before[c - 1].prev_type != FT_SILENT as u8 {
                    put(&mut enc, &mut ch.m.ftype[ch.prev_type as usize], FT_COPY, &mut self.stats.bits_flags);
                    ch.copy_frame(&before[c - 1], n);
                    self.es[c].err = self.es[c - 1].err;
                    self.stats.frames[FT_COPY] += 1;
                    continue;
                }
                encode_frame(&self.p, &self.t, &mut self.an, ch, &mut self.es[c], &mut self.s, &mut self.stats, &x[c], f, &mut enc);
            }
            self.ch.iter_mut().for_each(|ch| ch.slide(n));
        }
        let out = enc.finish();
        self.stats.bytes += out.len() as u64;
        out
    }
}

/// Codes a symbol and books its cost.
#[inline]
fn put(enc: &mut RangeEncoder, cdf: &mut crate::range::Cdf, s: usize, bits: &mut f64) {
    *bits += cdf.cost(s) as f64;
    enc.encode(cdf, s);
}

/// Is frame `f` background: no louder than twice the quiet tenth of the four seconds around it, and at least
/// 20 dB under the loud tenth. A stretch with no pauses has no background by this rule.
fn is_background(levels: &[f32], f: usize, near: &mut Vec<f32>) -> bool {
    let Some(&level) = levels.get(f) else { return false };
    near.clear();
    near.extend(levels[f.saturating_sub(100)..(f + 101).min(levels.len())].iter().filter(|&&v| v > 0.0));
    if near.len() < 20 {
        return false;
    }
    near.sort_unstable_by(f32::total_cmp);
    let (quiet, loud) = (near[near.len() / 10], near[near.len() * 9 / 10]);
    level < 2.0 * quiet && level < 0.1 * loud
}

/// The energy of what `coef` leaves of xb[from .. from + n].
fn resid_energy(xb: &[i32], from: usize, n: usize, coef: &[i32; MAX_ORDER], order: usize) -> f64 {
    let mut e = 0.0;
    for i in from..from + n {
        let mut s = 0i64;
        for j in 0..order {
            s += coef[j] as i64 * xb[i - 1 - j] as i64;
        }
        let d = (xb[i] as i64 - ((s + (1 << (COEF_SHIFT - 1))) >> COEF_SHIFT)) as f64;
        e += d * d;
    }
    e
}

#[allow(clippy::too_many_arguments)]
fn encode_frame(p: &Params, t: &Tuning, an: &mut lpc::Analysis, ch: &mut Chan, es: &mut EncChan, s: &mut Scratch, st: &mut EncStats, x: &[i32], frame: usize, enc: &mut RangeEncoder) {
    let (n, order, h) = (p.frame_len as usize, p.order as usize, ch.hist);
    let start = frame * n;
    let look = n / 4;
    // xb[i] = x[start - pre + i]: xb[pre] is the frame's first sample, xb[order + i] lies under exc[i] and y[i]
    let pre = h + order;
    s.xb.clear();
    s.xb.extend((0..pre + n + look).map(|i| {
        let j = (start + i) as isize - pre as isize;
        if j >= 0 {
            x.get(j as usize).copied().unwrap_or(0)
        } else {
            0
        }
    }));
    let xb = &s.xb;
    let prev = ch.prev_type as usize;
    if xb[pre..pre + n].iter().all(|&v| v == 0) {
        put(enc, &mut ch.m.ftype[prev], FT_SILENT, &mut st.bits_flags);
        st.frames[FT_SILENT] += 1;
        ch.silent_frame(n);
        es.err = [0.0; MAX_ORDER];
        return;
    }

    // ---- the short-term predictor: a new one, unless the old one does nearly as well for free
    let new_idx = match an.reflection(&xb[pre - look..pre + n + look], order) {
        Some(k) => an.quantize(&k, order),
        None => [0i16; MAX_ORDER],
    };
    let new_coef = lpc::coefs_from_idx(&new_idx, order);
    let mut syms = [0usize; MAX_ORDER];
    let mut cost = 0.0f64;
    for i in 0..order {
        syms[i] = (new_idx[i] as i32 - ch.idx[i] as i32).rem_euclid(lpc::alphabet(i) as i32) as usize;
        cost += ch.m.lpc[i].cost(syms[i]) as f64;
    }
    let e_new = resid_energy(xb, pre, n, &new_coef, order);
    let e_old = resid_energy(xb, pre, n, &ch.coef, order);
    // half a bit per sample for every halving of the residual's amplitude; coarse steps gain less than that
    let gain_bits = 0.5 * n as f64 * ((e_old + 1.0) / (e_new + 1.0)).log2() * if p.lossless { 1.0 } else { 0.7 };
    let repeat = gain_bits < cost;
    if !repeat {
        ch.idx = new_idx;
        ch.coef = new_coef;
    }

    // ---- what the predictor leaves of the input, over the past as well: the signal the pitch is searched in
    s.d.clear();
    s.d.extend((0..h + n).map(|i| {
        let xi = order + i;
        let mut acc = 0i64;
        for j in 0..order {
            acc += ch.coef[j] as i64 * xb[xi - 1 - j] as i64;
        }
        (xb[xi] as i64 - ((acc + (1 << (COEF_SHIFT - 1))) >> COEF_SHIFT)) as f32
    }));
    let d = &s.d;

    // ---- the frame's pitch: the lag at which the residual best repeats. Searched below about 1 kHz (two running
    // means), where a voice is periodic and its breath noise is weak
    let (lag_min, lag_max) = (p.lag_min as usize, p.lag_max as usize);
    let span = (p.sample_rate as usize / 2000).max(2);
    running_sum(d, &mut s.low1, span);
    running_sum(&s.low1, &mut s.low, span);
    let low = &s.low;
    let e0: f64 = low[h..h + n].iter().map(|&v| v as f64 * v as f64).sum();
    s.score.clear();
    let mut best = 0.0f64;
    for lag in lag_min..=lag_max {
        let (mut c, mut e) = (0.0f64, 0.0f64);
        for i in h..h + n {
            let v = low[i - lag] as f64;
            c += low[i] as f64 * v;
            e += v * v;
        }
        let sc = if c > 0.0 && e > 0.0 { c * c / e } else { 0.0 };
        best = best.max(sc);
        s.score.push(sc);
    }
    // the shortest lag that does nearly as well as the best one: a lag of two periods scores like one period
    let mut open_lag = lag_min;
    let mut rho = 0.0;
    if best > 0.0 && e0 > 0.0 {
        let sc = &s.score;
        let pick = (0..sc.len())
            .find(|&i| sc[i] >= 0.85 * best && (i == 0 || sc[i] >= sc[i - 1]) && (i + 1 == sc.len() || sc[i] >= sc[i + 1]))
            .unwrap_or(0);
        open_lag = lag_min + pick;
        rho = (sc[pick] / e0).sqrt();
    }
    let voiced = rho > t.voiced;
    let ft = if voiced { FT_VOICED } else { FT_UNVOICED };

    // ---- speech keeps its waveform. The background between words is noise to the ear: it takes far coarser steps,
    // and where they leave zeros the decoder puts noise of the size those samples had (0.25, 0.19 or 0.12 of the
    // step, by how much of a Laplacian falls inside a step this coarse)
    let background = t.background > 1.0 && !voiced && is_background(&es.levels, frame, &mut s.near);
    let (coarser, fill) = match (background, t.ratio * t.background) {
        (false, _) => (1.0, 0),
        (true, r) if r < 3.3 => (t.background, 3),
        (true, r) if r < 5.5 => (t.background, 2),
        (true, _) => (t.background, 1),
    };
    st.frames[ft] += 1;
    st.background += background as u64;
    st.repeats += repeat as u64;

    put(enc, &mut ch.m.ftype[prev], ft, &mut st.bits_flags);
    ch.prev_type = ft as u8;
    put(enc, &mut ch.m.repeat, repeat as usize, &mut st.bits_flags);
    if !repeat {
        for i in 0..order {
            put(enc, &mut ch.m.lpc[i], syms[i], &mut st.bits_lpc);
        }
    }
    put(enc, &mut ch.m.fill, fill, &mut st.bits_flags);

    // the predictor with its poles pulled in: what the past rounding errors are fed back through
    let mut shape = [0.0f32; MAX_ORDER];
    let mut g = 1.0f32;
    for (w, &c) in shape.iter_mut().zip(&ch.coef).take(order) {
        g *= t.gamma;
        *w = c as f32 / (1 << COEF_SHIFT) as f32 * g;
    }
    let (lo, hi) = p.sample_range();
    let mut lag = open_lag;
    for sf in 0..SUBFRAMES {
        let (a, b) = p.sub(sf);
        let (sa, sb) = (h + a, h + b);
        let rdd: f64 = d[sa..sb].iter().map(|&v| v as f64 * v as f64).sum();
        let prev_lag = lag;
        // ---- long-term prediction: the lag near the last one and the taps that leave the least
        let mut left = rdd;
        let (mut gain, mut tap_shape) = (0usize, 0usize);
        if voiced {
            // the excitation a decoder has; inside this subframe it is not built yet and the residual stands in
            let past = |j: usize| -> f64 {
                if j < sa {
                    ch.exc[j] as f64
                } else {
                    d[j] as f64
                }
            };
            let (from, to) = if sf == 0 {
                (open_lag.saturating_sub(6).max(lag_min), (open_lag + 6).min(lag_max))
            } else {
                (lag.saturating_sub(LAG_DELTA_BIAS as usize).max(lag_min), (lag + LAG_DELTAS - 1 - LAG_DELTA_BIAS as usize).min(lag_max))
            };
            let mut top = 0.0;
            let mut found = lag;
            for cand in from..=to {
                let (mut c, mut e) = (0.0f64, 0.0f64);
                for i in sa..sb {
                    let v = past(i - cand);
                    c += d[i] as f64 * v;
                    e += v * v;
                }
                if c > 0.0 && e > 0.0 && c * c / e > top {
                    top = c * c / e;
                    found = cand;
                }
            }
            lag = found.clamp(from, to);
            if top > 0.0 {
                let mut cj = [0.0f64; 3];
                let mut rjk = [[0.0f64; 3]; 3];
                for i in sa..sb {
                    let v = [past(i - lag - 1), past(i - lag), past(i - lag + 1)];
                    for j in 0..3 {
                        cj[j] += d[i] as f64 * v[j];
                        for k in 0..3 {
                            rjk[j][k] += v[j] * v[k];
                        }
                    }
                }
                for gi in 1..LTP_GAINS {
                    for sh in 0..2 {
                        let tq = ltp_taps(gi, sh);
                        let bq = [tq[0] as f64 / 128.0, tq[1] as f64 / 128.0, tq[2] as f64 / 128.0];
                        let mut e = rdd;
                        for j in 0..3 {
                            e -= 2.0 * bq[j] * cj[j];
                            for k in 0..3 {
                                e += bq[j] * bq[k] * rjk[j][k];
                            }
                        }
                        if e < left {
                            (left, gain, tap_shape) = (e, gi, sh);
                        }
                    }
                }
                // not worth its bits unless it takes a real part of the residual away
                if left > 0.97 * rdd {
                    (left, gain) = (rdd, 0);
                }
            }
        }
        let taps = if gain > 0 { ltp_taps(gain, tap_shape) } else { [0; 3] };

        // ---- the step
        let step_idx = if p.lossless {
            0u8
        } else {
            let sigma = (left.max(0.0) / (sb - sa) as f64).sqrt();
            let delta = (t.ratio * coarser * sigma).max(t.floor);
            let want = (4.0 * delta.log2()).round().clamp(0.0, (STEP_SYMS - 1) as f64) as u8;
            // a step one notch (1.5 dB) off is not worth the bits of a change
            if want.abs_diff(ch.step) <= t.hold {
                ch.step
            } else {
                want
            }
        };
        put(enc, &mut ch.m.step[(sf > 0) as usize], (step_idx as usize + STEP_SYMS - ch.step as usize) % STEP_SYMS, &mut st.bits_step);
        ch.step = step_idx;
        let sq = step_q4(step_idx);
        let fa = fill_amp(sq, fill);
        if voiced {
            if sf == 0 {
                enc.bits((lag - lag_min) as u32, p.lag_bits());
                st.bits_pitch += p.lag_bits() as f64;
            } else {
                put(enc, &mut ch.m.lag, (lag as i64 - prev_lag as i64 + LAG_DELTA_BIAS as i64) as usize, &mut st.bits_pitch);
            }
            put(enc, &mut ch.m.gain, gain, &mut st.bits_pitch);
            if gain > 0 {
                put(enc, &mut ch.m.shape, tap_shape, &mut st.bits_pitch);
            }
        }

        // ---- the samples
        let inv = 16.0 / sq as f32;
        for pos in sa..sb {
            let xi = xb[order + pos];
            let short = ch.st(pos, order);
            let lt = ch.lt(pos, lag, &taps);
            let target = xi as i64 - short - lt;
            let u = if p.lossless {
                target as i32
            } else {
                let mut fb = 0.0f32;
                for k in 0..order {
                    fb += shape[k] * es.err[k];
                }
                let q = (target as f32 + fb) * inv;
                let m = (q.abs() + 0.5 - t.dead).floor().clamp(0.0, 1e9);
                (if q < 0.0 { -m } else { m }) as i32
            };
            encode_residual(ch, enc, u);
            let y = ch.commit(pos, short, lt, u, sq, fa, lo, hi);
            if !p.lossless {
                es.err.copy_within(0..order - 1, 1);
                es.err[0] = (y - xi) as f32;
            }
        }
    }
}

#[inline]
fn encode_residual(ch: &mut Chan, enc: &mut RangeEncoder, u: i32) {
    let (k, ctx) = ch.res_ctx();
    let mag = u.unsigned_abs().min(i32::MAX as u32);
    let top = mag >> k;
    if top < TOK_ESC {
        enc.encode(&mut ch.m.tok[ctx], top as usize);
    } else {
        enc.encode(&mut ch.m.tok[ctx], TOK_ESC as usize);
        let v = top - TOK_ESC + 1;
        let nb = 31 - v.leading_zeros();
        enc.bits(nb, 5);
        enc.bits_long(v - (1 << nb), nb);
    }
    if k > 0 {
        enc.bits_long(mag & ((1 << k) - 1), k);
    }
    if mag != 0 {
        enc.bits((u < 0) as u32, 1);
    }
    ch.res_update(mag);
}
