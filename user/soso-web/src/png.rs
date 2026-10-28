//! Decodificador PNG mínimo (8 bits RGB/RGBA, filtros estándar).

use alloc::vec;
use alloc::vec::Vec;
use miniz_oxide::inflate::decompress_to_vec_zlib;

pub const MAX_FILE_BYTES: usize = 256 * 1024;
pub const MAX_SIDE: u32 = 512;
pub const MAX_IMAGES: usize = 8;

#[derive(Clone, Debug)]
pub struct DecodedImage {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

pub fn decode_png(data: &[u8]) -> Result<DecodedImage, ()> {
    if data.len() < 8 || &data[0..8] != [137, 80, 78, 71, 13, 10, 26, 10] {
        return Err(());
    }
    let mut pos = 8usize;
    let mut width = 0u32;
    let mut height = 0u32;
    let mut bit_depth = 0u8;
    let mut color_type = 0u8;
    let mut idat: Vec<u8> = Vec::new();
    while pos + 12 <= data.len() {
        let len = u32::from_be_bytes(data[pos..pos + 4].try_into().map_err(|_| ())?) as usize;
        let typ = &data[pos + 4..pos + 8];
        pos += 8;
        if pos + len + 4 > data.len() {
            return Err(());
        }
        let chunk = &data[pos..pos + len];
        pos += len + 4;
        if typ == b"IHDR" {
            if len < 13 {
                return Err(());
            }
            width = u32::from_be_bytes(chunk[0..4].try_into().map_err(|_| ())?);
            height = u32::from_be_bytes(chunk[4..8].try_into().map_err(|_| ())?);
            bit_depth = chunk[8];
            color_type = chunk[9];
            if bit_depth != 8 || (color_type != 2 && color_type != 6) {
                return Err(());
            }
            if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
                return Err(());
            }
        } else if typ == b"IDAT" {
            idat.extend_from_slice(chunk);
        } else if typ == b"IEND" {
            break;
        }
    }
    if width == 0 || height == 0 || idat.is_empty() {
        return Err(());
    }
    let bpp = if color_type == 6 { 4usize } else { 3usize };
    let raw = decompress_to_vec_zlib(&idat).map_err(|_| ())?;
    let row_bytes = width as usize * bpp;
    let expected = height as usize * (1 + row_bytes);
    if raw.len() < expected {
        return Err(());
    }
    let mut rgba = vec![0u8; width as usize * height as usize * 4];
    let mut prev = vec![0u8; row_bytes];
    let mut rp = 0usize;
    for y in 0..height as usize {
        let filter = raw[rp];
        rp += 1;
        let row = &raw[rp..rp + row_bytes];
        rp += row_bytes;
        let mut cur = vec![0u8; row_bytes];
        for i in 0..row_bytes {
            cur[i] = match filter {
                0 => row[i],
                1 => row[i].wrapping_add(if i >= bpp { cur[i - bpp] } else { 0 }),
                2 => row[i].wrapping_add(prev[i]),
                3 => {
                    let a = if i >= bpp { cur[i - bpp] as u16 } else { 0 };
                    let b = prev[i] as u16;
                    row[i].wrapping_add(((a + b) / 2) as u8)
                }
                4 => {
                    let a = if i >= bpp { cur[i - bpp] as i16 } else { 0 };
                    let b = prev[i] as i16;
                    let c = if i >= bpp { prev[i - bpp] as i16 } else { 0 };
                    row[i].wrapping_add(paeth(a, b, c) as u8)
                }
                _ => return Err(()),
            };
        }
        prev.clone_from_slice(&cur);
        for x in 0..width as usize {
            let dst = (y * width as usize + x) * 4;
            if color_type == 6 {
                rgba[dst] = cur[x * 4];
                rgba[dst + 1] = cur[x * 4 + 1];
                rgba[dst + 2] = cur[x * 4 + 2];
                rgba[dst + 3] = cur[x * 4 + 3];
            } else {
                rgba[dst] = cur[x * 3];
                rgba[dst + 1] = cur[x * 3 + 1];
                rgba[dst + 2] = cur[x * 3 + 2];
                rgba[dst + 3] = 255;
            }
        }
    }
    Ok(DecodedImage {
        w: width,
        h: height,
        rgba,
    })
}

fn paeth(a: i16, b: i16, c: i16) -> i16 {
    let p = a + b - c;
    let pa = (p - a).abs();
    let pb = (p - b).abs();
    let pc = (p - c).abs();
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_1x1_png() {
        // PNG 1x1 rojo opaco (generado mínimo)
        let png: &[u8] = &[
            0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00,
            0x00, 0x90, 0x77, 0x53, 0xde, 0x00, 0x00, 0x00, 0x0c, 0x49, 0x44, 0x41, 0x54, 0x08,
            0xd7, 0x63, 0xf8, 0xcf, 0xc0, 0x00, 0x00, 0x03, 0x01, 0x01, 0x00, 0x18, 0xdd, 0x8d,
            0xb4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
        ];
        let img = decode_png(png).expect("png");
        assert_eq!((img.w, img.h), (1, 1));
        assert_eq!(img.rgba.len(), 4);
        assert_eq!(img.rgba[0], 255);
    }
}
