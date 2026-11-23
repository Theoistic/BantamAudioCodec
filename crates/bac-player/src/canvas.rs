//! A software canvas: 0x00RRGGBB pixels, shapes with smooth edges. Nothing here knows the platform. Text is not
//! drawn here: the window lets the system draw it onto the same pixels.

/// A rectangle in pixels: its top-left corner and size.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Rect { x, y, w, h }
    }
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
    pub fn right(&self) -> f32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
    pub fn cx(&self) -> f32 {
        self.x + self.w * 0.5
    }
    pub fn cy(&self) -> f32 {
        self.y + self.h * 0.5
    }
    /// Smaller by `d` on every side.
    pub fn inset(&self, d: f32) -> Rect {
        Rect::new(self.x + d, self.y + d, (self.w - 2.0 * d).max(0.0), (self.h - 2.0 * d).max(0.0))
    }
}

/// Colour `c` over pixel `d` at opacity `a` (0..1).
#[inline]
pub fn blend(d: u32, c: u32, a: f32) -> u32 {
    if a >= 0.999 {
        return c;
    }
    if a <= 0.001 {
        return d;
    }
    let a = (a * 256.0) as u32;
    let ch = |shift: u32| {
        let (dv, cv) = ((d >> shift) & 255, (c >> shift) & 255);
        ((dv * (256 - a) + cv * a) >> 8) << shift
    };
    ch(16) | ch(8) | ch(0)
}

/// `a` to `b` by `t` (0..1).
pub fn mix(a: u32, b: u32, t: f32) -> u32 {
    blend(a, b, t.clamp(0.0, 1.0))
}

/// A drawing surface over a buffer of `w` x `h` pixels.
pub struct Canvas<'a> {
    pub w: i32,
    pub h: i32,
    pub px: &'a mut [u32],
    /// pixels outside it are never touched: x0, y0, x1, y1
    pub clip: (i32, i32, i32, i32),
}

impl<'a> Canvas<'a> {
    pub fn new(w: i32, h: i32, px: &'a mut [u32]) -> Self {
        Canvas { w, h, px, clip: (0, 0, w, h) }
    }

    /// The pixel box of a rectangle grown by `pad`, cut to the clip.
    fn bounds(&self, x0: f32, y0: f32, x1: f32, y1: f32, pad: f32) -> (i32, i32, i32, i32) {
        (
            ((x0 - pad).floor() as i32).max(self.clip.0),
            ((y0 - pad).floor() as i32).max(self.clip.1),
            ((x1 + pad).ceil() as i32).min(self.clip.2),
            ((y1 + pad).ceil() as i32).min(self.clip.3),
        )
    }

    #[inline]
    fn put(&mut self, x: i32, y: i32, c: u32, a: f32) {
        let i = (y * self.w + x) as usize;
        self.px[i] = blend(self.px[i], c, a);
    }

    /// A plain rectangle on whole pixels.
    pub fn fill(&mut self, r: Rect, c: u32) {
        let (x0, y0, x1, y1) = self.bounds(r.x.round(), r.y.round(), r.right().round(), r.bottom().round(), 0.0);
        for y in y0..y1 {
            let row = (y * self.w) as usize;
            if x1 > x0 {
                self.px[row + x0 as usize..row + x1 as usize].fill(c);
            }
        }
    }

    /// Runs `cover` (the share of the pixel inside the shape, from the signed distance of its centre) over a box.
    fn shade(&mut self, b: (i32, i32, i32, i32), c: u32, alpha: f32, dist: impl Fn(f32, f32) -> f32) {
        for y in b.1..b.3 {
            for x in b.0..b.2 {
                let a = (0.5 - dist(x as f32 + 0.5, y as f32 + 0.5)).clamp(0.0, 1.0) * alpha;
                if a > 0.0 {
                    self.put(x, y, c, a);
                }
            }
        }
    }

    /// A filled rectangle with round corners.
    pub fn rrect(&mut self, r: Rect, radius: f32, c: u32, alpha: f32) {
        let radius = radius.min(r.w * 0.5).min(r.h * 0.5).max(0.0);
        let (cx, cy, hx, hy) = (r.cx(), r.cy(), r.w * 0.5 - radius, r.h * 0.5 - radius);
        let dist = move |x: f32, y: f32| {
            let (qx, qy) = ((x - cx).abs() - hx, (y - cy).abs() - hy);
            (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - radius
        };
        // the rows between the corners are full spans: only their two ends need the distance
        let top = (r.y + radius).ceil() as i32 + 1;
        let bottom = ((r.bottom() - radius).floor() as i32 - 1).max(top);
        let b = self.bounds(r.x, r.y, r.right(), r.bottom(), 1.0);
        self.shade((b.0, b.1, b.2, b.3.min(top)), c, alpha, dist);
        self.shade((b.0, b.1.max(bottom), b.2, b.3), c, alpha, dist);
        let ix0 = (r.x.ceil() as i32 + 1).clamp(b.0, b.2.max(b.0));
        let ix1 = (r.right().floor() as i32 - 1).clamp(ix0, b.2.max(ix0));
        for y in b.1.max(top)..b.3.min(bottom) {
            if ix1 > ix0 {
                if alpha >= 0.999 {
                    let row = (y * self.w) as usize;
                    self.px[row + ix0 as usize..row + ix1 as usize].fill(c);
                } else {
                    for x in ix0..ix1 {
                        self.put(x, y, c, alpha);
                    }
                }
            }
        }
        self.shade((b.0, b.1.max(top), b.2.min(ix0), b.3.min(bottom)), c, alpha, dist);
        self.shade((b.0.max(ix1), b.1.max(top), b.2, b.3.min(bottom)), c, alpha, dist);
    }

    /// The outline of a rectangle with round corners, `width` thick, inside the rectangle.
    pub fn stroke(&mut self, r: Rect, radius: f32, width: f32, c: u32, alpha: f32) {
        let radius = radius.min(r.w * 0.5).min(r.h * 0.5).max(0.0);
        let (cx, cy, hx, hy) = (r.cx(), r.cy(), r.w * 0.5 - radius, r.h * 0.5 - radius);
        let half = width * 0.5;
        let dist = move |x: f32, y: f32| {
            let (qx, qy) = ((x - cx).abs() - hx, (y - cy).abs() - hy);
            let d = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - radius;
            (d + half).abs() - half
        };
        let b = self.bounds(r.x, r.y, r.right(), r.bottom(), 1.0);
        let edge = (radius.max(width) + 2.0).ceil() as i32;
        // the four bands that hold the outline; the middle is left alone. No pixel is in two bands
        let top = r.y as i32 + edge;
        let bottom = (r.bottom() as i32 - edge).max(top);
        let left = r.x as i32 + edge;
        let right = (r.right() as i32 - edge).max(left);
        self.shade((b.0, b.1, b.2, b.3.min(top)), c, alpha, dist);
        self.shade((b.0, b.1.max(bottom), b.2, b.3), c, alpha, dist);
        let (y0, y1) = (b.1.max(top), b.3.min(bottom));
        self.shade((b.0, y0, b.2.min(left), y1), c, alpha, dist);
        self.shade((b.0.max(right), y0, b.2, y1), c, alpha, dist);
    }

    pub fn circle(&mut self, cx: f32, cy: f32, radius: f32, c: u32) {
        let b = self.bounds(cx - radius, cy - radius, cx + radius, cy + radius, 1.0);
        self.shade(b, c, 1.0, |x, y| ((x - cx).powi(2) + (y - cy).powi(2)).sqrt() - radius);
    }

    /// A circle's outline, `width` thick, centred on the radius.
    pub fn ring(&mut self, cx: f32, cy: f32, radius: f32, width: f32, c: u32) {
        self.arc(cx, cy, radius, width, 0.0, 360.0, c);
    }

    /// Part of a circle's outline from angle `from` to `to` in degrees, clockwise from the top.
    #[allow(clippy::too_many_arguments)]
    pub fn arc(&mut self, cx: f32, cy: f32, radius: f32, width: f32, from: f32, to: f32, c: u32) {
        let b = self.bounds(cx - radius, cy - radius, cx + radius, cy + radius, width + 1.0);
        let whole = to - from >= 359.9;
        self.shade(b, c, 1.0, |x, y| {
            let (dx, dy) = (x - cx, y - cy);
            if !whole {
                let ang = dx.atan2(-dy).to_degrees().rem_euclid(360.0);
                let inside = if from <= to { ang >= from && ang <= to } else { ang >= from || ang <= to };
                if !inside {
                    return 1.0;
                }
            }
            ((dx * dx + dy * dy).sqrt() - radius).abs() - width * 0.5
        });
    }

    /// A line with round ends.
    pub fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, width: f32, c: u32) {
        let b = self.bounds(x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1), width + 1.0);
        let (vx, vy) = (x1 - x0, y1 - y0);
        let len2 = (vx * vx + vy * vy).max(1e-6);
        self.shade(b, c, 1.0, |x, y| {
            let t = (((x - x0) * vx + (y - y0) * vy) / len2).clamp(0.0, 1.0);
            ((x - x0 - t * vx).powi(2) + (y - y0 - t * vy).powi(2)).sqrt() - width * 0.5
        });
    }

    /// A filled polygon (convex or not), smoothed by 16 samples per pixel.
    pub fn poly(&mut self, pts: &[(f32, f32)], c: u32) {
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for &(x, y) in pts {
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
        }
        let b = self.bounds(x0, y0, x1, y1, 1.0);
        let inside = |x: f32, y: f32| {
            let mut odd = false;
            let mut j = pts.len() - 1;
            for i in 0..pts.len() {
                let ((xi, yi), (xj, yj)) = (pts[i], pts[j]);
                if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
                    odd = !odd;
                }
                j = i;
            }
            odd
        };
        for y in b.1..b.3 {
            for x in b.0..b.2 {
                let mut n = 0;
                for sy in 0..4 {
                    for sx in 0..4 {
                        n += inside(x as f32 + (sx as f32 + 0.5) / 4.0, y as f32 + (sy as f32 + 0.5) / 4.0) as u32;
                    }
                }
                if n > 0 {
                    self.put(x, y, c, n as f32 / 16.0);
                }
            }
        }
    }

    /// A vertical line one pixel wide; `dash` > 0 draws that many pixels on, as many off.
    pub fn vline(&mut self, x: f32, y0: f32, y1: f32, c: u32, alpha: f32, dash: i32) {
        let x = x.round() as i32;
        if x < self.clip.0 || x >= self.clip.2 {
            return;
        }
        for y in (y0.round() as i32).max(self.clip.1)..(y1.round() as i32).min(self.clip.3) {
            if dash == 0 || (y / dash) % 2 == 0 {
                self.put(x, y, c, alpha);
            }
        }
    }
}

/// The pixels as a PNG file (not compressed: for snapshots, which are looked at and thrown away).
pub fn png(w: u32, h: u32, px: &[u32]) -> Vec<u8> {
    let mut raw = Vec::with_capacity((w as usize * 3 + 1) * h as usize);
    for row in px.chunks(w as usize).take(h as usize) {
        raw.push(0);
        for &p in row {
            raw.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, p as u8]);
        }
    }
    // zlib: a header, stored blocks of at most 65,535 bytes, the Adler-32 of the data
    let mut z = vec![0x78, 0x01];
    let blocks = raw.chunks(65_535).count();
    for (i, block) in raw.chunks(65_535).enumerate() {
        z.push((i + 1 == blocks) as u8);
        z.extend_from_slice(&(block.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(block.len() as u16)).to_le_bytes());
        z.extend_from_slice(block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &v in &raw {
        a = (a + v as u32) % 65_521;
        b = (b + a) % 65_521;
    }
    z.extend_from_slice(&(b << 16 | a).to_be_bytes());
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut chunk = |kind: &[u8; 4], body: &[u8]| {
        out.extend_from_slice(&(body.len() as u32).to_be_bytes());
        let from = out.len();
        out.extend_from_slice(kind);
        out.extend_from_slice(body);
        let crc = bac::wire::crc32(&out[from..]);
        out.extend_from_slice(&crc.to_be_bytes());
    };
    let mut head = Vec::new();
    head.extend_from_slice(&w.to_be_bytes());
    head.extend_from_slice(&h.to_be_bytes());
    head.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(b"IHDR", &head);
    chunk(b"IDAT", &z);
    chunk(b"IEND", &[]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_stay_inside_the_clip_and_cover_their_middle() {
        let mut px = vec![0u32; 64 * 64];
        let mut c = Canvas::new(64, 64, &mut px);
        c.clip = (8, 8, 40, 40);
        c.rrect(Rect::new(0.0, 0.0, 64.0, 64.0), 12.0, 0xFFFFFF, 1.0);
        c.stroke(Rect::new(2.0, 2.0, 60.0, 60.0), 6.0, 2.0, 0xFF0000, 1.0);
        c.line(0.0, 0.0, 63.0, 63.0, 3.0, 0x00FF00);
        c.poly(&[(10.0, 50.0), (60.0, 50.0), (30.0, 5.0)], 0x0000FF);
        c.arc(32.0, 32.0, 20.0, 2.0, 300.0, 60.0, 0x123456);
        for y in 0..64 {
            for x in 0..64 {
                let inside = (8..40).contains(&x) && (8..40).contains(&y);
                assert!(inside || px[y * 64 + x] == 0, "({x}, {y}) outside the clip was drawn");
            }
        }
        assert_ne!(px[30 * 64 + 12], 0);
    }

    #[test]
    fn a_round_corner_is_smooth() {
        let mut px = vec![0u32; 32 * 32];
        Canvas::new(32, 32, &mut px).rrect(Rect::new(4.0, 4.0, 24.0, 24.0), 8.0, 0xFFFFFF, 1.0);
        assert_eq!(px[4 * 32 + 4], 0);
        assert_eq!(px[16 * 32 + 16], 0xFFFFFF);
        // along the corners' arcs some pixels are in between, and only there
        let between = px.iter().filter(|&&p| !matches!(p & 255, 0 | 255)).count();
        assert!((8..80).contains(&between), "{between} pixels in between");
    }

    #[test]
    fn png_has_its_parts() {
        let p = png(3, 2, &[0xFF0000, 0x00FF00, 0x0000FF, 0, 0xFFFFFF, 0x808080]);
        assert_eq!(&p[1..4], b"PNG");
        assert_eq!(&p[12..16], b"IHDR");
        assert_eq!(&p[p.len() - 8..p.len() - 4], b"IEND");
    }
}
