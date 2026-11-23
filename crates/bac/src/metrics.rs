//! How far a decoded signal is from its source, in numbers. They rank settings and catch mistakes; they do not
//! replace listening. All of them compare waveforms, so they are fair between codecs that try to keep the waveform
//! and unfair to codecs that only keep how it sounds.

/// What `compare` found.
#[derive(Clone, Debug, Default)]
pub struct Report {
    /// samples the test signal was moved to line up with the reference (codecs with a delay)
    pub offset: isize,
    /// signal over error, whole file, dB
    pub snr: f64,
    /// the mean over frames with signal of each frame's SNR, held to -10..35 dB: quiet parts count like loud ones
    pub seg_snr: f64,
    /// log-spectral distance: the RMS difference of the two power spectra in dB, mean over frames with signal
    pub lsd: f64,
    /// noise-to-mask ratio, dB: the error against a rough masking threshold of the reference in critical bands.
    /// Below 0 the error is mostly hidden by the signal; every 6 dB up is clearly worse.
    pub nmr: f64,
    /// band distance: the RMS difference of the two signals' energies per critical band in dB, mean over frames
    /// with signal. It hears the spectral envelope and not the waveform, so noise replaced by other noise of the
    /// same colour and level counts as close, which SNR, LSD and NMR all count as error.
    pub band: f64,
    /// frames with signal that the means are over
    pub frames: usize,
}

/// Compares `test` with `reference` (both mono, full scale 2^(bits-1)). `search`: the largest delay to look for.
pub fn compare(reference: &[i32], test: &[i32], rate: u32, bits: u8, search: usize) -> Report {
    let scale = 1.0 / (1u64 << (bits - 1)) as f64;
    let offset = align(reference, test, search);
    // the stretch both signals cover once `test` is moved
    let (r0, t0) = if offset >= 0 { (0usize, offset as usize) } else { ((-offset) as usize, 0usize) };
    let n = reference.len().saturating_sub(r0).min(test.len().saturating_sub(t0));
    let x: Vec<f64> = reference[r0..r0 + n].iter().map(|&v| v as f64 * scale).collect();
    let y: Vec<f64> = test[t0..t0 + n].iter().map(|&v| v as f64 * scale).collect();
    let mut rep = Report { offset, ..Default::default() };
    let (sig, err) = x.iter().zip(&y).fold((0.0, 0.0), |(s, e), (&a, &b)| (s + a * a, e + (a - b) * (a - b)));
    rep.snr = 10.0 * (sig / err.max(1e-20)).log10();

    // frames of about 32 ms, half overlapped
    let size = ((rate as f64 * 0.032) as usize).next_power_of_two();
    let hop = size / 2;
    let win: Vec<f64> = (0..size).map(|i| 0.5 - 0.5 * (2.0 * std::f64::consts::PI * (i as f64 + 0.5) / size as f64).cos()).collect();
    let bands = bark_bands(size, rate);
    let (mut seg, mut lsd, mut nmr, mut band) = (0.0, 0.0, 0.0, 0.0);
    let mut i = 0;
    while i + size <= n {
        let (fx, fy) = (&x[i..i + size], &y[i..i + size]);
        i += hop;
        let (s, e) = fx.iter().zip(fy).fold((0.0, 0.0), |(s, e), (&a, &b)| (s + a * a, e + (a - b) * (a - b)));
        // frames below -60 dB full scale are not speech and are left out
        if s / (size as f64) < 1e-6 {
            continue;
        }
        rep.frames += 1;
        seg += (10.0 * (s / e.max(1e-20)).log10()).clamp(-10.0, 35.0);
        let px = power(fx, &win);
        let py = power(fy, &win);
        let diff: Vec<f64> = fx.iter().zip(fy).map(|(a, b)| b - a).collect();
        let pe = power(&diff, &win);
        // spectra are compared down to 50 dB below the frame's strongest bin
        let floor = px.iter().cloned().fold(0.0, f64::max) * 1e-5 + 1e-14;
        let d2: f64 = px.iter().zip(&py).map(|(&a, &b)| (10.0 * ((a + floor) / (b + floor)).log10()).powi(2)).sum::<f64>() / px.len() as f64;
        lsd += d2.sqrt();
        nmr += 10.0 * noise_to_mask(&px, &pe, &bands).max(1e-10).log10();
        // band energies are compared down to 50 dB below the frame's strongest band
        let (ex, ey): (Vec<f64>, Vec<f64>) = bands.iter().map(|&(a, b)| (px[a..b].iter().sum::<f64>(), py[a..b].iter().sum::<f64>())).unzip();
        let low = ex.iter().cloned().fold(0.0, f64::max) * 1e-5 + 1e-14;
        band += (ex.iter().zip(&ey).map(|(&a, &b)| (10.0 * ((a + low) / (b + low)).log10()).powi(2)).sum::<f64>() / ex.len() as f64).sqrt();
    }
    let k = rep.frames.max(1) as f64;
    (rep.seg_snr, rep.lsd, rep.nmr, rep.band) = (seg / k, lsd / k, nmr / k, band / k);
    rep
}

/// The shift of `test` against `reference` with the largest correlation, over the first 10 seconds at most.
fn align(reference: &[i32], test: &[i32], search: usize) -> isize {
    if search == 0 {
        return 0;
    }
    let n = reference.len().min(test.len()).min(160_000);
    let mut best = (f64::MIN, 0isize);
    for o in -(search as isize)..=search as isize {
        let mut c = 0.0;
        // every 4th sample is plenty to find the peak
        for i in (search..n.saturating_sub(search)).step_by(4) {
            c += reference[i] as f64 * test[(i as isize + o) as usize] as f64;
        }
        if c > best.0 {
            best = (c, o);
        }
    }
    best.1
}

/// The power spectrum of a windowed frame, bins 0 ..= size/2.
fn power(x: &[f64], win: &[f64]) -> Vec<f64> {
    let n = x.len();
    let mut re: Vec<f64> = x.iter().zip(win).map(|(a, w)| a * w).collect();
    let mut im = vec![0.0; n];
    fft(&mut re, &mut im);
    (0..=n / 2).map(|k| re[k] * re[k] + im[k] * im[k]).collect()
}

/// In-place radix-2 FFT; the length is a power of two.
pub fn fft(re: &mut [f64], im: &mut [f64]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -2.0 * std::f64::consts::PI / len as f64;
        let (wr, wi) = (ang.cos(), ang.sin());
        for start in (0..n).step_by(len) {
            let (mut cr, mut ci) = (1.0, 0.0);
            for k in 0..len / 2 {
                let (a, b) = (start + k, start + k + len / 2);
                let (tr, ti) = (re[b] * cr - im[b] * ci, re[b] * ci + im[b] * cr);
                (re[b], im[b]) = (re[a] - tr, im[a] - ti);
                (re[a], im[a]) = (re[a] + tr, im[a] + ti);
                (cr, ci) = (cr * wr - ci * wi, cr * wi + ci * wr);
            }
        }
        len <<= 1;
    }
}

/// The spectrum's bins grouped into critical bands: for each band its first bin and one past its last.
fn bark_bands(size: usize, rate: u32) -> Vec<(usize, usize)> {
    let bark = |f: f64| 13.0 * (0.00076 * f).atan() + 3.5 * (f / 7500.0).powi(2).atan();
    let bins = size / 2 + 1;
    let mut bands: Vec<(usize, usize)> = Vec::new();
    let mut from = 1;
    for k in 1..=bins {
        let edge = k == bins || bark(k as f64 * rate as f64 / size as f64).floor() > bark(from as f64 * rate as f64 / size as f64).floor();
        if edge && k > from {
            bands.push((from, k));
            from = k;
        }
    }
    bands
}

/// The mean over bands of error energy over masked threshold. The threshold of a band is the signal's energy
/// spread over the neighbouring bands (falling 25 dB per band downwards and 10 dB per band upwards), 10 dB down.
fn noise_to_mask(px: &[f64], pe: &[f64], bands: &[(usize, usize)]) -> f64 {
    let sum = |p: &[f64], (a, b): (usize, usize)| p[a..b].iter().sum::<f64>();
    let sig: Vec<f64> = bands.iter().map(|&b| sum(px, b)).collect();
    let mut total = 0.0;
    for (i, &b) in bands.iter().enumerate() {
        let mut spread = 0.0;
        for (j, &s) in sig.iter().enumerate() {
            let dz = i as f64 - j as f64;
            spread += s * 10f64.powf(if dz >= 0.0 { -dz } else { 2.5 * dz });
        }
        // nothing is audible below about -96 dB full scale in a band
        let threshold = (spread * 0.1).max(1e-9 * (b.1 - b.0) as f64);
        total += sum(pe, b) / threshold;
    }
    total / bands.len() as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone() -> Vec<i32> {
        (0..32_000).map(|i| (8000.0 * (i as f64 * 0.11).sin() + 3000.0 * (i as f64 * 0.37).sin()) as i32).collect()
    }

    #[test]
    fn more_noise_is_worse_on_every_scale() {
        let x = tone();
        let noisy = |amp: i32| -> Vec<i32> {
            let mut s = 1u32;
            x.iter()
                .map(|&v| {
                    s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    v + ((s >> 16) as i32 % (2 * amp + 1)) - amp
                })
                .collect()
        };
        let a = compare(&x, &noisy(20), 16_000, 16, 0);
        let b = compare(&x, &noisy(400), 16_000, 16, 0);
        assert!(a.snr > b.snr + 20.0 && a.seg_snr > b.seg_snr && a.lsd < b.lsd && a.nmr < b.nmr - 20.0 && a.band < b.band, "{a:?} {b:?}");
        assert!(a.frames > 100);
        let same = compare(&x, &x, 16_000, 16, 0);
        assert!(same.snr > 150.0 && same.lsd < 1e-6);
    }

    #[test]
    fn a_delay_is_found() {
        let x = tone();
        let mut y = vec![0; 37];
        y.extend_from_slice(&x);
        let r = compare(&x, &y, 16_000, 16, 100);
        assert_eq!(r.offset, 37);
        assert!(r.snr > 150.0);
    }
}
