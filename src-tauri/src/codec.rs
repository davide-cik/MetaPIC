//! Format detection, decoding and encoding.

use anyhow::{Context, Result, anyhow, bail};
use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::{DynamicImage, ImageFormat, RgbImage};
use serde::{Deserialize, Serialize};
use std::io::Cursor;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Jpeg,
    Png,
    Webp,
    Avif,
    Jxl,
    Heic,
    Tiff,
    Gif,
    Bmp,
    Ico,
    Qoi,
}

impl Format {
    pub fn extension(self) -> &'static str {
        match self {
            Format::Jpeg => "jpg",
            Format::Png => "png",
            Format::Webp => "webp",
            Format::Avif => "avif",
            Format::Jxl => "jxl",
            Format::Heic => "heic",
            Format::Tiff => "tif",
            Format::Gif => "gif",
            Format::Bmp => "bmp",
            Format::Ico => "ico",
            Format::Qoi => "qoi",
        }
    }

    fn image_format(self) -> Option<ImageFormat> {
        Some(match self {
            Format::Jpeg => ImageFormat::Jpeg,
            Format::Png => ImageFormat::Png,
            Format::Webp => ImageFormat::WebP,
            Format::Tiff => ImageFormat::Tiff,
            Format::Gif => ImageFormat::Gif,
            Format::Bmp => ImageFormat::Bmp,
            Format::Ico => ImageFormat::Ico,
            Format::Qoi => ImageFormat::Qoi,
            Format::Avif | Format::Jxl | Format::Heic => return None,
        })
    }
}

/// Detects the format from the first bytes of the file (extension is ignored).
pub fn sniff(bytes: &[u8]) -> Option<Format> {
    // ISO-BMFF container: AVIF and HEIC share it, the brands tell them apart.
    if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
        let box_len = u32::from_be_bytes(bytes[0..4].try_into().unwrap()) as usize;
        let brands = &bytes[8..box_len.clamp(12, bytes.len())];
        if brands.as_chunks::<4>().0.iter().any(|b| b == b"avif" || b == b"avis") {
            return Some(Format::Avif);
        }
        if brands
            .as_chunks::<4>().0.iter()
            .any(|b| matches!(b, b"heic" | b"heix" | b"hevc" | b"hevx" | b"heim" | b"heis" | b"mif1"))
        {
            return Some(Format::Heic);
        }
        return None;
    }
    if bytes.starts_with(&[0xFF, 0x0A]) || bytes.starts_with(b"\0\0\0\x0CJXL \r\n\x87\n") {
        return Some(Format::Jxl);
    }
    Some(match image::guess_format(bytes).ok()? {
        ImageFormat::Jpeg => Format::Jpeg,
        ImageFormat::Png => Format::Png,
        ImageFormat::WebP => Format::Webp,
        ImageFormat::Tiff => Format::Tiff,
        ImageFormat::Gif => Format::Gif,
        ImageFormat::Bmp => Format::Bmp,
        ImageFormat::Ico => Format::Ico,
        ImageFormat::Qoi => Format::Qoi,
        _ => return None,
    })
}

pub fn decode(bytes: &[u8]) -> Result<(DynamicImage, Format)> {
    let format = sniff(bytes).ok_or_else(|| anyhow!("formato non riconosciuto"))?;
    let img = match format {
        Format::Avif => decode_avif(bytes)?,
        Format::Jxl => {
            let decoder = jxl_oxide::integration::JxlDecoder::new(Cursor::new(bytes))?;
            DynamicImage::from_decoder(decoder)?
        }
        Format::Heic => bail!("HEIC/HEIF non ancora supportato in lettura"),
        other => image::load_from_memory_with_format(bytes, other.image_format().unwrap())?,
    };
    Ok((img, format))
}

#[cfg(not(feature = "avif-read"))]
fn decode_avif(_: &[u8]) -> Result<DynamicImage> {
    bail!("lettura AVIF non inclusa in questa build")
}

#[cfg(feature = "avif-read")]
fn decode_avif(bytes: &[u8]) -> Result<DynamicImage> {
    use avif_decode::Image;
    use image::ImageBuffer;
    let image = avif_decode::Decoder::from_avif(bytes)
        .map_err(|e| anyhow!("AVIF: {e}"))?
        .to_image()
        .map_err(|e| anyhow!("AVIF: {e}"))?;

    macro_rules! buffer {
        ($img:expr, $sub:ty, $variant:ident) => {{
            let (buf, w, h) = $img.into_contiguous_buf();
            let raw: Vec<$sub> = bytemuck::cast_slice(&buf).to_vec();
            let buffer = ImageBuffer::from_raw(w as u32, h as u32, raw)
                .ok_or_else(|| anyhow!("AVIF: buffer non valido"))?;
            DynamicImage::$variant(buffer)
        }};
    }

    Ok(match image {
        Image::Rgb8(img) => buffer!(img, u8, ImageRgb8),
        Image::Rgba8(img) => buffer!(img, u8, ImageRgba8),
        Image::Gray8(img) => buffer!(img, u8, ImageLuma8),
        Image::Rgb16(img) => buffer!(img, u16, ImageRgb16),
        Image::Rgba16(img) => buffer!(img, u16, ImageRgba16),
        Image::Gray16(img) => buffer!(img, u16, ImageLuma16),
    })
}

pub struct EncodeOptions {
    /// 1–100, ignored by lossless-only formats.
    pub quality: u8,
    pub lossless: bool,
    /// AVIF encoder speed, 1 (slow, smaller) – 10 (fast).
    pub speed: u8,
    /// Raw EXIF block for encoders that embed it themselves (AVIF).
    pub exif: Option<Vec<u8>>,
}

pub fn encode(img: &DynamicImage, format: Format, opts: &EncodeOptions) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let has_alpha = img.color().has_alpha();
    let (w, h) = (img.width(), img.height());
    match format {
        Format::Jpeg => {
            out = encode_mozjpeg(&flatten_on_white(img), opts.quality)?;
        }
        Format::Png => {
            let encoder = PngEncoder::new_with_quality(&mut out, CompressionType::Best, FilterType::Adaptive);
            img.write_with_encoder(encoder)?;
        }
        Format::Webp => {
            let (rgba, rgb);
            let encoder = if has_alpha {
                rgba = img.to_rgba8();
                webp::Encoder::from_rgba(&rgba, w, h)
            } else {
                rgb = img.to_rgb8();
                webp::Encoder::from_rgb(&rgb, w, h)
            };
            let memory = if opts.lossless {
                encoder.encode_lossless()
            } else {
                encoder.encode(opts.quality as f32)
            };
            out = memory.to_vec();
        }
        Format::Avif => {
            let rgba = img.to_rgba8();
            let pixels = rgb::FromSlice::as_rgba(rgba.as_raw().as_slice());
            let quality = if opts.lossless { 100.0 } else { opts.quality as f32 };
            let mut encoder = ravif::Encoder::new()
                .with_quality(quality)
                .with_alpha_quality(quality)
                .with_speed(opts.speed.clamp(1, 10));
            if let Some(exif) = &opts.exif {
                encoder = encoder.with_exif(exif.clone());
            }
            out = encoder
                .encode_rgba(ravif::Img::new(pixels, w as usize, h as usize))
                .context("AVIF")?
                .avif_file;
        }
        Format::Jxl => {
            use zune_core::{bit_depth::BitDepth, colorspace::ColorSpace, options::EncoderOptions};
            let (raw, space) = if has_alpha {
                (img.to_rgba8().into_raw(), ColorSpace::RGBA)
            } else {
                (img.to_rgb8().into_raw(), ColorSpace::RGB)
            };
            let options = EncoderOptions::new(w as usize, h as usize, space, BitDepth::Eight);
            zune_jpegxl::JxlSimpleEncoder::new(&raw, options)
                .encode(&mut out)
                .map_err(|e| anyhow!("JPEG XL: {e:?}"))?;
        }
        Format::Ico => {
            // ICO entries are at most 256×256.
            let icon = if w > 256 || h > 256 { img.thumbnail(256, 256) } else { img.clone() };
            DynamicImage::ImageRgba8(icon.to_rgba8()).write_to(&mut Cursor::new(&mut out), ImageFormat::Ico)?;
        }
        Format::Gif => {
            DynamicImage::ImageRgba8(img.to_rgba8()).write_to(&mut Cursor::new(&mut out), ImageFormat::Gif)?;
        }
        Format::Tiff | Format::Bmp | Format::Qoi => {
            let normalized = if has_alpha {
                DynamicImage::ImageRgba8(img.to_rgba8())
            } else {
                DynamicImage::ImageRgb8(img.to_rgb8())
            };
            normalized.write_to(&mut Cursor::new(&mut out), format.image_format().unwrap())?;
        }
        Format::Heic => bail!("la scrittura HEIC non è ancora supportata"),
    }
    Ok(out)
}

/// mozjpeg with its defaults: progressive scans, optimized Huffman tables and
/// trellis quantization, typically 10–20% smaller than baseline at equal quality.
fn encode_mozjpeg(rgb: &RgbImage, quality: u8) -> Result<Vec<u8>> {
    // libjpeg reports fatal errors by unwinding.
    std::panic::catch_unwind(|| -> std::io::Result<Vec<u8>> {
        let mut comp = mozjpeg::Compress::new(mozjpeg::ColorSpace::JCS_RGB);
        comp.set_size(rgb.width() as usize, rgb.height() as usize);
        comp.set_quality(quality.clamp(1, 100) as f32);
        let mut started = comp.start_compress(Vec::new())?;
        started.write_scanlines(rgb.as_raw())?;
        started.finish()
    })
    .map_err(|_| anyhow!("JPEG: errore interno di mozjpeg"))?
    .context("JPEG")
}

/// JPEG has no alpha: composite transparent pixels on white instead of black.
fn flatten_on_white(img: &DynamicImage) -> RgbImage {
    if !img.color().has_alpha() {
        return img.to_rgb8();
    }
    let rgba = img.to_rgba8();
    RgbImage::from_fn(img.width(), img.height(), |x, y| {
        let [r, g, b, a] = rgba.get_pixel(x, y).0;
        let blend = |c: u8| ((c as u32 * a as u32 + 255 * (255 - a as u32)) / 255) as u8;
        image::Rgb([blend(r), blend(g), blend(b)])
    })
}
