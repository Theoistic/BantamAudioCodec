//! The program's icon: a dark tile with a few bars of sound in the previewer's teal. Drawn by the canvas, and
//! written as the `.ico` and `.res` files that put it on `bac.exe`. Nothing here knows the platform.

use crate::canvas::{Canvas, Rect};

/// the sizes inside the icon file
pub const SIZES: [u32; 5] = [16, 24, 32, 48, 256];

/// The icon at `size` x `size` pixels as 0xAARRGGBB, top row first, colours not multiplied by their cover.
pub fn render(size: u32) -> Vec<u32> {
    let n = (size * size) as usize;
    let (mut colour, mut cover) = (vec![0u32; n], vec![0u32; n]);
    let s = size as f32;
    for (px, tile, bar) in [(&mut colour, 0x24222Eu32, 0x2FE3C4u32), (&mut cover, 0xFFFFFF, 0xFFFFFF)] {
        let mut c = Canvas::new(size as i32, size as i32, px);
        c.rrect(Rect::new(0.0, 0.0, s, s), s * 0.22, tile, 1.0);
        for (i, part) in [0.3f32, 0.62, 0.9, 0.5, 0.75, 0.28].iter().enumerate() {
            let x = s * (0.19 + 0.125 * i as f32);
            c.line(x, s * 0.5 - s * 0.36 * part, x, s * 0.5 + s * 0.36 * part, (s * 0.075).max(1.5), bar);
        }
    }
    // the picture was drawn over black, which multiplied it by its cover: divide that out again
    colour
        .iter()
        .zip(&cover)
        .map(|(&c, &a)| {
            let a = a & 0xFF;
            let ch = |shift: u32| (((c >> shift) & 0xFF) * 255).checked_div(a).unwrap_or(0).min(255) << shift;
            a << 24 | ch(16) | ch(8) | ch(0)
        })
        .collect()
}

/// One picture as an icon file holds it: a bitmap header of twice the height, the rows bottom first as B, G, R, A,
/// then a mask of one bit per pixel (all zero: the cover channel does the work).
fn image(size: u32) -> Vec<u8> {
    let px = render(size);
    let mut out = Vec::new();
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(size as i32).to_le_bytes());
    out.extend_from_slice(&(2 * size as i32).to_le_bytes());
    out.extend_from_slice(&[1, 0, 32, 0]);
    out.extend_from_slice(&[0u8; 24]);
    for row in px.chunks(size as usize).rev() {
        for &p in row {
            out.extend_from_slice(&p.to_le_bytes());
        }
    }
    out.resize(out.len() + (size.div_ceil(32) * 4 * size) as usize, 0);
    out
}

/// A directory entry without its last field (the file offset, or the resource number).
fn entry(size: u32, len: usize) -> Vec<u8> {
    let side = if size >= 256 { 0 } else { size as u8 };
    let mut e = vec![side, side, 0, 0, 1, 0, 32, 0];
    e.extend_from_slice(&(len as u32).to_le_bytes());
    e
}

/// The icon as an `.ico` file.
pub fn ico() -> Vec<u8> {
    let images: Vec<Vec<u8>> = SIZES.iter().map(|&s| image(s)).collect();
    let mut out = vec![0, 0, 1, 0];
    out.extend_from_slice(&(SIZES.len() as u16).to_le_bytes());
    let mut at = 6 + 16 * SIZES.len();
    for (s, im) in SIZES.iter().zip(&images) {
        out.extend(entry(*s, im.len()));
        out.extend_from_slice(&(at as u32).to_le_bytes());
        at += im.len();
    }
    for im in &images {
        out.extend_from_slice(im);
    }
    out
}

/// One resource of a `.res` file: a 32-byte header with numbered type and name, the data, padding to 4 bytes.
fn resource(out: &mut Vec<u8>, kind: u16, name: u16, flags: u16, data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&32u32.to_le_bytes());
    for id in [kind, name] {
        out.extend_from_slice(&0xFFFFu16.to_le_bytes());
        out.extend_from_slice(&id.to_le_bytes());
    }
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&0x0409u16.to_le_bytes());
    out.extend_from_slice(&[0u8; 8]);
    out.extend_from_slice(data);
    out.resize(out.len().div_ceil(4) * 4, 0);
}

/// A compiled resource file with the icon as group 1: the first icon group is what Windows shows for a program.
pub fn res() -> Vec<u8> {
    const RT_ICON: u16 = 3;
    const RT_GROUP_ICON: u16 = 14;
    let mut out = Vec::new();
    // every .res file starts with an empty resource
    resource(&mut out, 0, 0, 0, &[]);
    let mut group = vec![0, 0, 1, 0];
    group.extend_from_slice(&(SIZES.len() as u16).to_le_bytes());
    for (i, &s) in SIZES.iter().enumerate() {
        let im = image(s);
        resource(&mut out, RT_ICON, i as u16 + 1, 0x1010, &im);
        group.extend(entry(s, im.len()));
        group.extend_from_slice(&(i as u16 + 1).to_le_bytes());
    }
    resource(&mut out, RT_GROUP_ICON, 1, 0x1030, &group);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tile_with_clear_corners_and_teal_bars() {
        for size in [16u32, 48, 256] {
            let px = render(size);
            let at = |x: u32, y: u32| px[(y * size + x) as usize];
            assert!(at(0, 0) >> 24 < 60 && at(size - 1, size - 1) >> 24 < 60, "{size}: corners");
            assert_eq!(at(size / 2, size / 2) >> 24, 255);
            // the bars: pixels much greener than the tile (thin bars at 16 pixels never reach the full colour)
            let teal = px.iter().filter(|&&p| p >> 24 == 255 && (p >> 8) & 255 > 110).count();
            assert!(teal > (size * size / 40) as usize && teal < (size * size / 2) as usize, "{size}: {teal} teal pixels");
        }
    }

    #[test]
    fn the_files_have_the_layout_windows_reads() {
        let f = ico();
        assert_eq!(&f[..4], &[0, 0, 1, 0]);
        let mut end = 0;
        for (i, &s) in SIZES.iter().enumerate() {
            let e = &f[6 + 16 * i..6 + 16 * (i + 1)];
            let (len, at) = (u32::from_le_bytes(e[8..12].try_into().unwrap()) as usize, u32::from_le_bytes(e[12..16].try_into().unwrap()) as usize);
            assert_eq!(u32::from_le_bytes(f[at..at + 4].try_into().unwrap()), 40);
            assert_eq!(len, (40 + s * s * 4 + s.div_ceil(32) * 4 * s) as usize);
            end = at + len;
        }
        assert_eq!(end, f.len());
        let r = res();
        let (mut p, mut kinds) = (0usize, Vec::new());
        while p < r.len() {
            let data = u32::from_le_bytes(r[p..p + 4].try_into().unwrap()) as usize;
            assert_eq!(u32::from_le_bytes(r[p + 4..p + 8].try_into().unwrap()), 32);
            kinds.push(u16::from_le_bytes([r[p + 10], r[p + 11]]));
            p += 32 + data.div_ceil(4) * 4;
        }
        assert_eq!(p, r.len());
        assert_eq!(kinds, [0, 3, 3, 3, 3, 3, 14]);
    }
}
