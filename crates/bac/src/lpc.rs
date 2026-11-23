//! The short-term predictor. A frame's predictor is stored as reflection coefficients on a coarse grid (the residual
//! barely feels their rounding, and they cannot describe an unstable filter). `coefs_from_idx` is the decoder's half
//! and is exact integer arithmetic; `Analysis` is the encoder's half and may use floats freely.

use crate::codec::{COEF_SHIFT, MAX_ORDER};

/// Half the number of grid points of reflection coefficient `i`: the grid index runs over -(L-1) ..= L-1.
pub fn grid_l(i: usize) -> i32 {
    match i {
        0..=1 => 32,
        2..=5 => 16,
        _ => 8,
    }
}

/// How many values the grid index of coefficient `i` can take.
pub fn alphabet(i: usize) -> usize {
    (2 * grid_l(i) - 1) as usize
}

/// The reflection coefficient at grid index `i` of a grid with half-size `l`, in Q15: with x = i/l it is
/// x (3 - x^2) / 2, which is dense near +-1 where the filter is most sensitive.
pub fn grid_k(i: i32, l: i32) -> i32 {
    let (a, l) = (i.unsigned_abs() as i64, l as i64);
    let l3 = l * l * l;
    let v = (((a * (3 * l * l - a * a)) << 14) + l3 / 2) / l3;
    if i < 0 {
        -(v as i32)
    } else {
        v as i32
    }
}

/// Grid indices -> the predictor's coefficients in Q14 (`coef[j]` multiplies the sample j + 1 back). The step-up
/// recursion in Q24 with rounding shifts; the result saturates at +-2^30.
pub fn coefs_from_idx(idx: &[i16; MAX_ORDER], order: usize) -> [i32; MAX_ORDER] {
    let mut a = [0i64; MAX_ORDER];
    let mut t = [0i64; MAX_ORDER];
    for m in 0..order {
        let k = grid_k(idx[m] as i32, grid_l(m)) as i64;
        for j in 0..m {
            t[j] = a[j] - ((k * a[m - 1 - j] + (1 << 14)) >> 15);
        }
        a[..m].copy_from_slice(&t[..m]);
        a[m] = k << 9;
    }
    let mut c = [0i32; MAX_ORDER];
    let down = 24 - COEF_SHIFT;
    for j in 0..order {
        c[j] = ((a[j] + (1 << (down - 1))) >> down).clamp(-(1 << 30), 1 << 30) as i32;
    }
    c
}

/// The encoder's analysis of one frame: window, autocorrelation, Levinson-Durbin, then the nearest grid points.
pub struct Analysis {
    win: Vec<f64>,
    lag_win: [f64; MAX_ORDER + 1],
    /// r[0] is raised by this factor: a noise floor that keeps the recursion well conditioned
    white: f64,
    /// the grid values of each half-size in use, as floats, from index -(L-1) up
    grids: [(i32, Vec<f64>); 3],
    buf: Vec<f64>,
}

impl Analysis {
    /// `len`: samples per analysis segment. Lossy coding smooths the spectrum a little (a 40 Hz lag window and a
    /// noise floor at -45 dB); lossless coding wants every bit of prediction and takes almost none.
    pub fn new(len: usize, rate: u32, lossless: bool) -> Analysis {
        let win = (0..len).map(|i| (std::f64::consts::PI * (i as f64 + 0.5) / len as f64).sin().powi(2)).collect();
        let (bw, white) = if lossless { (5.0, 1e-9) } else { (40.0, 3e-5) };
        let mut lag_win = [1.0; MAX_ORDER + 1];
        for (m, w) in lag_win.iter_mut().enumerate() {
            let a = 2.0 * std::f64::consts::PI * bw * m as f64 / rate as f64;
            *w = (-0.5 * a * a).exp();
        }
        let grid = |l: i32| (l, (-(l - 1)..l).map(|i| grid_k(i, l) as f64 / 32768.0).collect::<Vec<_>>());
        Analysis { win, lag_win, white, grids: [grid(32), grid(16), grid(8)], buf: Vec::new() }
    }

    /// The reflection coefficients of the windowed segment, or None for a segment of zeros.
    pub fn reflection(&mut self, seg: &[i32], order: usize) -> Option<[f64; MAX_ORDER]> {
        self.buf.clear();
        self.buf.extend(seg.iter().zip(&self.win).map(|(&v, &w)| v as f64 * w));
        let x = &self.buf;
        let mut r = [0.0f64; MAX_ORDER + 1];
        for (m, rm) in r.iter_mut().enumerate().take(order + 1) {
            *rm = x[m..].iter().zip(x.iter()).map(|(a, b)| a * b).sum::<f64>() * self.lag_win[m];
        }
        if r[0] <= 0.0 {
            return None;
        }
        r[0] *= 1.0 + self.white;
        let mut k = [0.0f64; MAX_ORDER];
        let mut a = [0.0f64; MAX_ORDER];
        let mut t = [0.0f64; MAX_ORDER];
        let mut e = r[0];
        for m in 0..order {
            let mut acc = r[m + 1];
            for j in 0..m {
                acc -= a[j] * r[m - j];
            }
            let km = (acc / e).clamp(-0.9999, 0.9999);
            k[m] = km;
            for j in 0..m {
                t[j] = a[j] - km * a[m - 1 - j];
            }
            a[..m].copy_from_slice(&t[..m]);
            a[m] = km;
            e *= 1.0 - km * km;
            if e <= r[0] * 1e-12 {
                break;
            }
        }
        Some(k)
    }

    /// The nearest grid index of every coefficient.
    pub fn quantize(&self, k: &[f64; MAX_ORDER], order: usize) -> [i16; MAX_ORDER] {
        let mut idx = [0i16; MAX_ORDER];
        for m in 0..order {
            let l = grid_l(m);
            let grid = &self.grids.iter().find(|g| g.0 == l).unwrap().1;
            let mut best = (f64::MAX, 0usize);
            for (i, &g) in grid.iter().enumerate() {
                let d = (g - k[m]).abs();
                if d < best.0 {
                    best = (d, i);
                }
            }
            idx[m] = (best.1 as i32 - (l - 1)) as i16;
        }
        idx
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_grid_rises_and_stays_inside_one() {
        for l in [8, 16, 32] {
            let g: Vec<i32> = (-(l - 1)..l).map(|i| grid_k(i, l)).collect();
            assert!(g.windows(2).all(|w| w[0] < w[1]));
            assert!(*g.last().unwrap() < 32768 && g[0] == -g.last().unwrap());
            assert_eq!(grid_k(0, l), 0);
        }
    }

    #[test]
    fn a_pure_tone_is_predicted() {
        // 16 kHz, a 440 Hz tone: two coefficients predict it, the rest stay near zero
        let x: Vec<i32> = (0..480).map(|i| (8000.0 * (i as f64 * 440.0 * 2.0 * std::f64::consts::PI / 16000.0).sin()) as i32).collect();
        let mut an = Analysis::new(480, 16000, true);
        let k = an.reflection(&x, 16).unwrap();
        let coef = coefs_from_idx(&an.quantize(&k, 16), 16);
        let (mut err, mut sig) = (0f64, 0f64);
        for i in 16..480 {
            let mut s = 0i64;
            for j in 0..16 {
                s += coef[j] as i64 * x[i - 1 - j] as i64;
            }
            let d = x[i] as i64 - ((s + (1 << 13)) >> 14);
            err += (d * d) as f64;
            sig += (x[i] as f64).powi(2);
        }
        // the grid stops at |k| = 0.9986, which bounds the gain on a pure tone
        assert!(10.0 * (sig / err).log10() > 20.0, "prediction gain {} dB", 10.0 * (sig / err).log10());
    }
}
