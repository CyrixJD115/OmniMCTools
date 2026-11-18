//! Image decoding: PNG (any color type / bit depth → RGBA8) and TGA
//! (true-color 24/32-bit and grayscale, raw or RLE — what resource packs
//! use).

use omt_art::Image;

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("png: {0}")]
    Png(String),
    #[error("tga: {0}")]
    Tga(&'static str),
}

pub fn decode(bytes: &[u8]) -> Result<Image, DecodeError> {
    if bytes.starts_with(b"\x89PNG") { png(bytes) } else { tga(bytes) }
}

pub fn png(bytes: &[u8]) -> Result<Image, DecodeError> {
    let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut r = dec.read_info().map_err(|e| DecodeError::Png(e.to_string()))?;
    let size = r.output_buffer_size().ok_or_else(|| DecodeError::Png("image too large".into()))?;
    let mut buf = vec![0; size];
    let info = r.next_frame(&mut buf).map_err(|e| DecodeError::Png(e.to_string()))?;
    let (w, h) = (info.width, info.height);
    let px = (w * h) as usize;
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf[..px * 4].to_vec(),
        png::ColorType::Rgb => buf[..px * 3].as_chunks::<3>().0.iter().flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => buf[..px * 2].as_chunks::<2>().0.iter().flat_map(|c| [c[0], c[0], c[0], c[1]]).collect(),
        png::ColorType::Grayscale => buf[..px].iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return Err(DecodeError::Png("palette not expanded".into())),
    };
    Ok(Image { w, h, rgba })
}

pub fn tga(b: &[u8]) -> Result<Image, DecodeError> {
    if b.len() < 18 {
        return Err(DecodeError::Tga("truncated header"));
    }
    let id_len = b[0] as usize;
    let cmap = b[1];
    let kind = b[2];
    let w = u16::from_le_bytes([b[12], b[13]]) as u32;
    let h = u16::from_le_bytes([b[14], b[15]]) as u32;
    let bpp = b[16] as usize;
    let desc = b[17];
    if cmap != 0 {
        return Err(DecodeError::Tga("color-mapped TGA not supported"));
    }
    let (gray, rle) = match kind {
        2 => (false, false),
        3 => (true, false),
        10 => (false, true),
        11 => (true, true),
        _ => return Err(DecodeError::Tga("unsupported image type")),
    };
    let bytes_pp = bpp / 8;
    if !matches!((gray, bytes_pp), (false, 3) | (false, 4) | (true, 1) | (true, 2)) {
        return Err(DecodeError::Tga("unsupported pixel depth"));
    }
    let n = (w * h) as usize;
    let mut src = 18 + id_len;
    let mut pixels: Vec<[u8; 4]> = Vec::with_capacity(n);
    let read_px = |p: &[u8]| -> [u8; 4] {
        match (gray, p.len()) {
            (true, 1) => [p[0], p[0], p[0], 255],
            (true, _) => [p[0], p[0], p[0], p[1]],
            (false, 3) => [p[2], p[1], p[0], 255],
            _ => [p[2], p[1], p[0], p[3]],
        }
    };
    while pixels.len() < n {
        if rle {
            let hdr = *b.get(src).ok_or(DecodeError::Tga("truncated data"))?;
            src += 1;
            let count = (hdr & 0x7f) as usize + 1;
            if hdr & 0x80 != 0 {
                let p = b.get(src..src + bytes_pp).ok_or(DecodeError::Tga("truncated data"))?;
                src += bytes_pp;
                let px = read_px(p);
                pixels.extend(std::iter::repeat_n(px, count.min(n - pixels.len())));
            } else {
                for _ in 0..count.min(n - pixels.len()) {
                    let p = b.get(src..src + bytes_pp).ok_or(DecodeError::Tga("truncated data"))?;
                    src += bytes_pp;
                    pixels.push(read_px(p));
                }
            }
        } else {
            let p = b.get(src..src + bytes_pp).ok_or(DecodeError::Tga("truncated data"))?;
            src += bytes_pp;
            pixels.push(read_px(p));
        }
    }
    // Default origin is bottom-left; bit 5 = top-left.
    let top_down = desc & 0x20 != 0;
    let mut rgba = Vec::with_capacity(n * 4);
    for y in 0..h as usize {
        let sy = if top_down { y } else { h as usize - 1 - y };
        for x in 0..w as usize {
            rgba.extend_from_slice(&pixels[sy * w as usize + x]);
        }
    }
    Ok(Image { w, h, rgba })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tga_header(kind: u8, w: u16, h: u16, bpp: u8, desc: u8) -> Vec<u8> {
        let mut v = vec![0u8, 0, kind, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        v.extend_from_slice(&w.to_le_bytes());
        v.extend_from_slice(&h.to_le_bytes());
        v.push(bpp);
        v.push(desc);
        v
    }

    #[test]
    fn tga_raw_and_rle_bottom_up() {
        // 2×2, 32-bit BGRA, bottom-left origin.
        let mut raw = tga_header(2, 2, 2, 32, 0);
        // bottom row: red, green; top row: blue, white
        raw.extend_from_slice(&[0, 0, 255, 255, 0, 255, 0, 255, 255, 0, 0, 255, 255, 255, 255, 255]);
        let img = tga(&raw).unwrap();
        assert_eq!(&img.rgba[0..4], &[0, 0, 255, 255], "top-left is blue");
        assert_eq!(&img.rgba[8..12], &[255, 0, 0, 255], "bottom-left is red");
        // RLE: 4 identical grey pixels, top-down.
        let mut rle = tga_header(11, 2, 2, 8, 0x20);
        rle.extend_from_slice(&[0x83, 77]);
        let img = tga(&rle).unwrap();
        assert!(img.rgba.as_chunks::<4>().0.iter().all(|p| *p == [77, 77, 77, 255]));
        assert!(tga(&[0; 5]).is_err());
    }

    #[test]
    fn png_roundtrip() {
        let mut out = Vec::new();
        {
            let mut e = png::Encoder::new(&mut out, 2, 1);
            e.set_color(png::ColorType::Rgb);
            e.set_depth(png::BitDepth::Eight);
            e.write_header().unwrap().write_image_data(&[1, 2, 3, 4, 5, 6]).unwrap();
        }
        let img = decode(&out).unwrap();
        assert_eq!(img.rgba, vec![1, 2, 3, 255, 4, 5, 6, 255]);
    }
}
