//! The entropy coder: a range coder over adaptive multi-symbol distributions, the scheme AV1 took from Daala. Nothing
//! about the distributions is stored: both sides start from the same table and move it towards each symbol they
//! code, so the tables follow the speaker and the recording. Integer arithmetic only, so every decoder gives the
//! same result.

/// probabilities are 15-bit integers
pub const PROB_BITS: u32 = 15;
pub const PROB_ONE: u32 = 1 << PROB_BITS;
/// the range is kept at or above this by shifting bytes out (in)
const TOP: u32 = 1 << 24;

/// An adaptive distribution over n symbols as cumulative counts: symbol s owns [c[s], c[s+1]) of [0, 32768).
#[derive(Clone, Debug)]
pub struct Cdf {
    c: Vec<u16>,
    /// symbols coded so far, saturating at 32: the first symbols move the table faster
    count: u8,
    /// no symbol's share falls below this
    floor: u16,
    base_rate: u8,
}

impl Cdf {
    /// The starting table: every symbol gets the floor, the rest is shared in proportion to `w`.
    pub fn from_weights(w: &[u32]) -> Cdf {
        let n = w.len();
        assert!((2..=512).contains(&n));
        let floor: u32 = if n <= 16 { 8 } else if n <= 64 { 4 } else { 2 };
        let total: u64 = w.iter().map(|&v| v as u64).sum();
        assert!(total > 0);
        let spare = (PROB_ONE - floor * n as u32) as u64;
        let mut c = Vec::with_capacity(n + 1);
        let mut acc = 0u64;
        for (i, &v) in w.iter().enumerate() {
            c.push((floor as u64 * i as u64 + spare * acc / total) as u16);
            acc += v as u64;
        }
        c.push(PROB_ONE as u16);
        let base_rate = 3 + (31 - (n as u32).leading_zeros()).min(2) as u8;
        Cdf { c, count: 0, floor: floor as u16, base_rate }
    }

    pub fn uniform(n: usize) -> Cdf {
        Cdf::from_weights(&vec![1; n])
    }

    /// A table for differences that wrap around n: most weight on 0, falling off to both sides.
    pub fn circular(n: usize) -> Cdf {
        let w: Vec<u32> = (0..n).map(|s| 1 + 64 / (1 + s.min(n - s) as u32)).collect();
        Cdf::from_weights(&w)
    }

    /// A table that halves from symbol to symbol.
    pub fn falling(n: usize) -> Cdf {
        let w: Vec<u32> = (0..n).map(|s| (4096u32 >> s.min(12)).max(1)).collect();
        Cdf::from_weights(&w)
    }

    /// Moves the table towards symbol `s`: every boundary goes a fraction of the way to where it would stand if `s`
    /// had all the probability the floors leave.
    #[inline]
    fn adapt(&mut self, s: usize) {
        let rate = self.base_rate as u32 + (self.count > 15) as u32 + (self.count > 31) as u32;
        let n = self.c.len() - 1;
        let f = self.floor as u32;
        for i in 1..n {
            let cur = self.c[i] as u32;
            let target = if i <= s { f * i as u32 } else { PROB_ONE - f * (n - i) as u32 };
            self.c[i] = if target >= cur { cur + ((target - cur) >> rate) } else { cur - ((cur - target) >> rate) } as u16;
        }
        if self.count < 32 {
            self.count += 1;
        }
    }

    /// What coding `s` would cost now, in bits (for the encoder's decisions).
    pub fn cost(&self, s: usize) -> f32 {
        -((self.c[s + 1] - self.c[s]) as f32 / PROB_ONE as f32).log2()
    }
}

/// Writes symbols and plain bits into bytes.
pub struct RangeEncoder {
    /// the low end of the interval; bit 32 is a carry into bytes already passed on
    low: u64,
    range: u32,
    /// the last byte passed on, held back with `pending - 1` bytes of 0xFF behind it until no carry can reach them
    cache: u8,
    pending: u64,
    /// false until the first byte, which is always zero and is not written
    started: bool,
    out: Vec<u8>,
}

impl Default for RangeEncoder {
    fn default() -> Self {
        Self::new()
    }
}

impl RangeEncoder {
    pub fn new() -> Self {
        RangeEncoder { low: 0, range: u32::MAX, cache: 0, pending: 1, started: false, out: Vec::new() }
    }

    fn shift_low(&mut self) {
        if (self.low as u32) < 0xFF00_0000 || (self.low >> 32) != 0 {
            let carry = (self.low >> 32) as u8;
            let mut b = self.cache;
            loop {
                if self.started {
                    self.out.push(b.wrapping_add(carry));
                } else {
                    self.started = true;
                }
                b = 0xFF;
                self.pending -= 1;
                if self.pending == 0 {
                    break;
                }
            }
            self.cache = (self.low >> 24) as u8;
        }
        self.pending += 1;
        self.low = (self.low & 0x00FF_FFFF) << 8;
    }

    #[inline]
    fn normalize(&mut self) {
        while self.range < TOP {
            self.range <<= 8;
            self.shift_low();
        }
    }

    /// Codes symbol `s` of `cdf` and adapts the table.
    #[inline]
    pub fn encode(&mut self, cdf: &mut Cdf, s: usize) {
        let (lo, hi) = (cdf.c[s] as u32, cdf.c[s + 1] as u32);
        let r = self.range >> PROB_BITS;
        self.low += r as u64 * lo as u64;
        // the last symbol also takes what the division left over
        self.range = if hi == PROB_ONE { self.range - r * lo } else { r * (hi - lo) };
        self.normalize();
        cdf.adapt(s);
    }

    /// The low `n` bits of `v` as they are, n <= 16.
    #[inline]
    pub fn bits(&mut self, v: u32, n: u32) {
        debug_assert!(n <= 16 && v >> n == 0);
        if n == 0 {
            return;
        }
        self.range >>= n;
        self.low += self.range as u64 * v as u64;
        self.normalize();
    }

    /// The low `n` bits of `v`, n <= 32.
    #[inline]
    pub fn bits_long(&mut self, v: u32, n: u32) {
        if n > 16 {
            self.bits(v >> 16, n - 16);
            self.bits(v & 0xFFFF, 16);
        } else {
            self.bits(v, n);
        }
    }

    /// Ends the stream. Of all values inside the final interval the one with the most zero bits at its end is
    /// written, and zero bytes at the end are dropped: the decoder reads zeros past the end.
    pub fn finish(mut self) -> Vec<u8> {
        let (lo, hi) = (self.low, self.low + self.range as u64 - 1);
        let mut v = lo;
        for nb in (1..=32).rev() {
            let mask = (1u64 << nb) - 1;
            let c = (lo + mask) & !mask;
            if c <= hi {
                v = c;
                break;
            }
        }
        self.low = v;
        for _ in 0..5 {
            self.shift_low();
        }
        while self.out.last() == Some(&0) {
            self.out.pop();
        }
        self.out
    }
}

/// Reads what `RangeEncoder` wrote. Never fails: past the end of the data it reads zeros, and damaged data gives
/// wrong symbols, not a panic (the container's checksum is what finds damage).
pub struct RangeDecoder<'a> {
    buf: &'a [u8],
    pos: usize,
    range: u32,
    code: u32,
}

impl<'a> RangeDecoder<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        let mut d = RangeDecoder { buf, pos: 0, range: u32::MAX, code: 0 };
        for _ in 0..4 {
            d.code = d.code << 8 | d.byte();
        }
        d
    }

    #[inline]
    fn byte(&mut self) -> u32 {
        let b = self.buf.get(self.pos).copied().unwrap_or(0);
        self.pos += 1;
        b as u32
    }

    #[inline]
    fn normalize(&mut self) {
        while self.range < TOP {
            self.range <<= 8;
            self.code = self.code << 8 | self.byte();
        }
    }

    /// The next symbol of `cdf`; adapts the table as the encoder did.
    #[inline]
    pub fn decode(&mut self, cdf: &mut Cdf) -> usize {
        let r = self.range >> PROB_BITS;
        let val = (self.code / r).min(PROB_ONE - 1);
        let c = &cdf.c;
        let mut s = 0;
        while c[s + 1] as u32 <= val {
            s += 1;
        }
        let (lo, hi) = (c[s] as u32, c[s + 1] as u32);
        self.code -= r * lo;
        self.range = if hi == PROB_ONE { self.range - r * lo } else { r * (hi - lo) };
        self.normalize();
        cdf.adapt(s);
        s
    }

    /// `n` plain bits, n <= 16.
    #[inline]
    pub fn bits(&mut self, n: u32) -> u32 {
        if n == 0 {
            return 0;
        }
        self.range >>= n;
        let v = (self.code / self.range).min((1u32 << n) - 1);
        self.code -= v * self.range;
        self.normalize();
        v
    }

    /// `n` plain bits, n <= 32.
    #[inline]
    pub fn bits_long(&mut self, n: u32) -> u32 {
        if n > 16 {
            let hi = self.bits(n - 16);
            hi << 16 | self.bits(16)
        } else {
            self.bits(n)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// a small generator, so that the tests need nothing
    pub(crate) struct Rng(pub u64);
    impl Rng {
        pub fn next(&mut self) -> u32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 32) as u32
        }
    }

    #[test]
    fn symbols_and_bits_come_back() {
        for seed in 1..40u64 {
            let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
            let sizes = [2usize, 3, 16, 31, 63, 96];
            // what to code: (table, symbol) or (bits, value)
            let n = 1 + (rng.next() % 3000) as usize;
            let skew = rng.next() % 4;
            let plan: Vec<(usize, u32, u32)> = (0..n)
                .map(|_| {
                    if rng.next().is_multiple_of(4) {
                        let nb = rng.next() % 33;
                        let v = if nb == 0 { 0 } else { rng.next() >> (32 - nb) };
                        (usize::MAX, nb, v)
                    } else {
                        let t = (rng.next() % sizes.len() as u32) as usize;
                        // skewed towards small symbols, as real data is
                        let mut s = rng.next() % sizes[t] as u32;
                        for _ in 0..skew {
                            s = s.min(rng.next() % sizes[t] as u32);
                        }
                        (t, s, 0)
                    }
                })
                .collect();
            let make = || sizes.iter().map(|&n| Cdf::uniform(n)).collect::<Vec<_>>();
            let mut tables = make();
            let mut enc = RangeEncoder::new();
            for &(t, a, b) in &plan {
                if t == usize::MAX {
                    enc.bits_long(b, a);
                } else {
                    enc.encode(&mut tables[t], a as usize);
                }
            }
            let bytes = enc.finish();
            let mut tables = make();
            let mut dec = RangeDecoder::new(&bytes);
            for (i, &(t, a, b)) in plan.iter().enumerate() {
                if t == usize::MAX {
                    assert_eq!(dec.bits_long(a), b, "seed {seed}, item {i}");
                } else {
                    assert_eq!(dec.decode(&mut tables[t]), a as usize, "seed {seed}, item {i}");
                }
            }
        }
    }

    #[test]
    fn a_likely_symbol_costs_little() {
        let mut t = Cdf::uniform(16);
        let mut enc = RangeEncoder::new();
        for _ in 0..10_000 {
            enc.encode(&mut t, 0);
        }
        // about 25 bytes: 18 while the table learns, then 0.0053 bits per symbol (the floor of the 15 other symbols)
        assert!(enc.finish().len() < 40);
    }

    #[test]
    fn no_share_falls_below_the_floor() {
        let mut rng = Rng(7);
        for n in [2usize, 5, 16, 64, 127] {
            let mut t = Cdf::circular(n);
            for i in 0..20_000 {
                let s = if i % 3 == 0 { (rng.next() % n as u32) as usize } else { n - 1 };
                t.adapt(s);
                assert!(t.c.windows(2).all(|w| w[1] - w[0] >= t.floor), "n {n}");
            }
        }
    }
}
