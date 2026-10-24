//! EXIF reading, filtering and editing (via little_exif).

use crate::codec::Format;
use anyhow::{Result, anyhow, bail};
use little_exif::exif_tag::ExifTag;
use little_exif::filetype::FileExtension;
use little_exif::ifd::ExifTagGroup;
use little_exif::metadata::Metadata;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetaMode {
    Keep,
    KeepNoGps,
    Strip,
}

/// A user edit on a text tag; an empty value removes the tag.
#[derive(Clone, Debug, Deserialize)]
pub struct Edit {
    pub tag: String,
    pub value: String,
}

#[derive(Serialize)]
pub struct TagView {
    pub name: String,
    pub group: String,
    pub value: String,
    pub editable: bool,
}

/// Text tags the editor exposes, in display order.
pub const EDITABLE: &[&str] = &[
    "ImageDescription",
    "Artist",
    "Copyright",
    "DateTimeOriginal",
    "Make",
    "Model",
    "Software",
];

fn text_tag(name: &str, value: String) -> Option<ExifTag> {
    Some(match name {
        "ImageDescription" => ExifTag::ImageDescription(value),
        "Artist" => ExifTag::Artist(value),
        "Copyright" => ExifTag::Copyright(value),
        "DateTimeOriginal" => ExifTag::DateTimeOriginal(value),
        "Make" => ExifTag::Make(value),
        "Model" => ExifTag::Model(value),
        "Software" => ExifTag::Software(value),
        _ => return None,
    })
}

pub fn file_extension(format: Format) -> Option<FileExtension> {
    Some(match format {
        Format::Jpeg => FileExtension::JPEG,
        Format::Png => FileExtension::PNG { as_zTXt_chunk: false },
        Format::Webp => FileExtension::WEBP,
        Format::Tiff => FileExtension::TIFF,
        Format::Jxl => FileExtension::JXL,
        Format::Avif | Format::Heic => FileExtension::HEIF,
        _ => return None,
    })
}

/// Whether metadata can be written into files of this format while converting.
pub fn writable(format: Format) -> bool {
    matches!(format, Format::Jpeg | Format::Png | Format::Webp | Format::Tiff | Format::Jxl | Format::Avif)
}

/// Whether existing files of this format can have their metadata edited.
/// (little_exif can't relocate the EXIF item inside an existing HEIF/AVIF.)
pub fn writable_in_place(format: Format) -> bool {
    writable(format) && format != Format::Avif
}

/// Reads EXIF from an encoded file. Malformed metadata is treated as absent.
pub fn read(bytes: &[u8], format: Format) -> Option<Metadata> {
    if matches!(format, Format::Avif | Format::Heic) {
        // little_exif misses EXIF items in many AVIF files; parse the container ourselves.
        if let Some(m) = crate::heif::exif_tiff(bytes).and_then(|tiff| parse_tiff_block(&tiff)) {
            return Some(m);
        }
    }
    parse(bytes.to_vec(), file_extension(format)?)
}

fn parse(buffer: Vec<u8>, ext: FileExtension) -> Option<Metadata> {
    std::panic::catch_unwind(|| Metadata::new_from_vec(&buffer, ext).ok())
        .ok()
        .flatten()
        .filter(|m| !is_empty(m))
}

/// Parses a bare TIFF EXIF block by wrapping it in a minimal JPEG APP1 segment.
fn parse_tiff_block(tiff: &[u8]) -> Option<Metadata> {
    let segment_len = u16::try_from(2 + 6 + tiff.len()).ok()?;
    let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE1];
    jpeg.extend_from_slice(&segment_len.to_be_bytes());
    jpeg.extend_from_slice(b"Exif\0\0");
    jpeg.extend_from_slice(tiff);
    jpeg.extend_from_slice(&[0xFF, 0xD9]);
    parse(jpeg, FileExtension::JPEG)
}

/// Applies an in-place metadata edit to an encoded file, returning the new bytes.
pub fn rewrite(mut bytes: Vec<u8>, format: Format, edits: &[Edit], remove_gps_tags: bool, strip_all: bool) -> Result<Vec<u8>> {
    if !writable_in_place(format) {
        bail!("questo formato non supporta la modifica dei metadati");
    }
    let mut m = match read(&bytes, format) {
        Some(m) if !strip_all => m,
        _ => Metadata::new(),
    };
    if remove_gps_tags {
        remove_gps(&mut m);
    }
    apply_edits(&mut m, edits);
    match format {
        // The TIFF directory holds the image layout too: edit it, never clear it.
        Format::Tiff => {
            let mut full = parse(bytes.clone(), FileExtension::TIFF).ok_or_else(|| anyhow!("TIFF non leggibile"))?;
            if strip_all {
                remove_descriptive(&mut full);
            }
            if remove_gps_tags {
                remove_gps(&mut full);
            }
            apply_edits(&mut full, edits);
            full.write_to_vec(&mut bytes, FileExtension::TIFF)?;
        }
        // embed_webp replaces the EXIF chunk itself.
        Format::Webp if !is_empty(&m) => embed(&mut bytes, format, &m)?,
        _ => {
            clear(&mut bytes, format)?;
            if !is_empty(&m) {
                embed(&mut bytes, format, &m)?;
            }
        }
    }
    Ok(bytes)
}

/// Baseline TIFF tags describing the pixel layout rather than the photo.
const STRUCTURAL: &[u16] = &[
    0x00FE, 0x0100, 0x0101, 0x0102, 0x0103, 0x0106, 0x0111, 0x0115, 0x0116, 0x0117, 0x011A, 0x011B, 0x011C, 0x0128,
    0x013D, 0x0140, 0x0142, 0x0143, 0x0144, 0x0145, 0x0152, 0x0153, 0x0201, 0x0202,
];

fn is_structural(tag: &ExifTag) -> bool {
    tag.get_group() == ExifTagGroup::GENERIC && STRUCTURAL.contains(&tag.as_u16())
}

fn remove_matching(m: &mut Metadata, pred: impl Fn(&ExifTag) -> bool) {
    let doomed: Vec<(u16, ExifTagGroup)> = m.into_iter().filter(|t| pred(t)).map(|t| (t.as_u16(), t.get_group())).collect();
    for (hex, group) in doomed {
        m.remove_tag_by_hex_group(hex, group);
    }
}

/// Keeps only the TIFF layout tags.
fn remove_descriptive(m: &mut Metadata) {
    remove_matching(m, |t| !is_structural(t));
}

pub fn is_empty(m: &Metadata) -> bool {
    m.into_iter().next().is_none()
}

pub fn orientation(m: &Metadata) -> Option<u8> {
    m.into_iter().find_map(|tag| match tag {
        ExifTag::Orientation(v) => v.first().map(|&o| o as u8),
        _ => None,
    })
}

pub fn remove_gps(m: &mut Metadata) {
    remove_matching(m, |t| t.get_group() == ExifTagGroup::GPS);
    m.remove_tag(ExifTag::GPSInfo(vec![]));
}

pub fn apply_edits(m: &mut Metadata, edits: &[Edit]) {
    for edit in edits {
        let value = edit.value.trim().to_string();
        let Some(tag) = text_tag(&edit.tag, value.clone()) else { continue };
        if value.is_empty() {
            m.remove_tag(tag);
        } else {
            m.set_tag(tag);
        }
    }
}

/// Builds the metadata for a converted image. Pixels are already rotated
/// upright and possibly resized, so orientation and dimensions are reset.
pub fn for_output(source: Option<Metadata>, mode: MetaMode, edits: &[Edit], width: u32, height: u32) -> Option<Metadata> {
    let mut m = match mode {
        MetaMode::Strip => Metadata::new(),
        _ => source.unwrap_or_default(),
    };
    // A TIFF source carries its own pixel layout, meaningless for the new file.
    remove_matching(&mut m, is_structural);
    if mode == MetaMode::KeepNoGps {
        remove_gps(&mut m);
    }
    apply_edits(&mut m, edits);
    if is_empty(&m) {
        return None;
    }
    if orientation(&m).is_some() {
        m.set_tag(ExifTag::Orientation(vec![1]));
    }
    let has = |m: &Metadata, hex: u16| m.get_tag_by_hex(hex, Some(ExifTagGroup::EXIF)).next().is_some();
    if has(&m, 0xa002) {
        m.set_tag(ExifTag::ExifImageWidth(vec![width]));
    }
    if has(&m, 0xa003) {
        m.set_tag(ExifTag::ExifImageHeight(vec![height]));
    }
    Some(m)
}

/// The bare TIFF-structured EXIF block, as WebP and HEIF containers store it.
pub fn tiff_block(m: &Metadata) -> Result<Vec<u8>> {
    let mut data = m.encode()?;
    if data.starts_with(b"Exif\0\0") {
        data.drain(..6);
    }
    Ok(data)
}

/// Writes metadata into an already encoded file buffer.
pub fn embed(bytes: &mut Vec<u8>, format: Format, m: &Metadata) -> Result<()> {
    if format == Format::Webp {
        // little_exif can't upgrade a simple (VP8/VP8L-only) WebP, which is what encoders emit.
        return embed_webp(bytes, &tiff_block(m)?);
    }
    if format == Format::Tiff {
        // In a TIFF the EXIF IFD0 is the image directory itself: merge into the
        // structural tags (size, strips…) instead of replacing them.
        let mut merged = parse(bytes.clone(), FileExtension::TIFF).ok_or_else(|| anyhow!("TIFF non leggibile"))?;
        for tag in m {
            if !is_structural(tag) && tag.is_writable() {
                merged.set_tag(tag.clone());
            }
        }
        merged.write_to_vec(bytes, FileExtension::TIFF)?;
        return Ok(());
    }
    let ext = file_extension(format).ok_or_else(|| anyhow!("metadati non supportati per questo formato"))?;
    m.write_to_vec(bytes, ext)?;
    Ok(())
}

/// Rewrites a WebP as the extended format (VP8X header) carrying `exif`,
/// replacing any EXIF chunk already present.
fn embed_webp(bytes: &mut Vec<u8>, exif: &[u8]) -> Result<()> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WEBP" {
        bail!("WebP non valido");
    }
    let mut chunks: Vec<([u8; 4], &[u8])> = Vec::new();
    let mut pos = 12;
    while pos + 8 <= bytes.len() {
        let id: [u8; 4] = bytes[pos..pos + 4].try_into().unwrap();
        let len = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let end = pos + 8 + len;
        if end > bytes.len() {
            bail!("WebP troncato");
        }
        chunks.push((id, &bytes[pos + 8..end]));
        pos = end + (len & 1);
    }

    let mut flags = 0u8;
    let mut size = None;
    for (id, data) in &chunks {
        match id {
            b"VP8X" if data.len() >= 10 => {
                flags = data[0];
                let w = u32::from_le_bytes([data[4], data[5], data[6], 0]) + 1;
                let h = u32::from_le_bytes([data[7], data[8], data[9], 0]) + 1;
                size = Some((w, h));
            }
            b"VP8L" if data.len() >= 5 && size.is_none() => {
                let bits = u32::from_le_bytes(data[1..5].try_into().unwrap());
                size = Some(((bits & 0x3FFF) + 1, ((bits >> 14) & 0x3FFF) + 1));
                if (bits >> 28) & 1 == 1 {
                    flags |= 0x10;
                }
            }
            b"VP8 " if data.len() >= 10 && size.is_none() => {
                let w = u16::from_le_bytes([data[6], data[7]]) as u32 & 0x3FFF;
                let h = u16::from_le_bytes([data[8], data[9]]) as u32 & 0x3FFF;
                size = Some((w, h));
            }
            b"ALPH" => flags |= 0x10,
            _ => {}
        }
    }
    let (w, h) = size.ok_or_else(|| anyhow!("WebP senza immagine"))?;
    flags |= 0x08; // EXIF present

    fn push_chunk(out: &mut Vec<u8>, id: &[u8], data: &[u8]) {
        out.extend_from_slice(id);
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(data);
        if data.len() % 2 == 1 {
            out.push(0);
        }
    }

    let mut vp8x = vec![flags, 0, 0, 0];
    vp8x.extend_from_slice(&(w - 1).to_le_bytes()[..3]);
    vp8x.extend_from_slice(&(h - 1).to_le_bytes()[..3]);

    let mut out = Vec::with_capacity(bytes.len() + exif.len() + 32);
    out.extend_from_slice(b"RIFF\0\0\0\0WEBP");
    push_chunk(&mut out, b"VP8X", &vp8x);
    let mut xmp = None;
    for (id, data) in &chunks {
        match id {
            b"VP8X" | b"EXIF" => {}
            b"XMP " => xmp = Some(*data), // must stay last
            _ => push_chunk(&mut out, id, data),
        }
    }
    push_chunk(&mut out, b"EXIF", exif);
    if let Some(data) = xmp {
        push_chunk(&mut out, b"XMP ", data);
    }
    let riff_len = (out.len() - 8) as u32;
    out[4..8].copy_from_slice(&riff_len.to_le_bytes());
    *bytes = out;
    Ok(())
}

/// Removes all metadata from an encoded file buffer.
pub fn clear(bytes: &mut Vec<u8>, format: Format) -> Result<()> {
    let ext = file_extension(format).ok_or_else(|| anyhow!("metadati non supportati per questo formato"))?;
    Metadata::clear_metadata(bytes, ext)?;
    Ok(())
}

/// Tags that only describe file layout, not the photo.
const HIDDEN: &[&str] = &[
    "ExifOffset",
    "GPSInfo",
    "InteropOffset",
    "StripOffsets",
    "StripByteCounts",
    "ThumbnailOffset",
    "ThumbnailLength",
];

pub fn list(m: &Metadata) -> Vec<TagView> {
    let mut tags: Vec<TagView> = m
        .into_iter()
        .filter_map(|tag| {
            // Debug output looks like `Artist("Mario")` or `FNumber([Rational ...])`.
            let debug = format!("{tag:?}");
            let (name, rest) = debug.split_once('(')?;
            if HIDDEN.contains(&name) {
                return None;
            }
            let mut value = rest.strip_suffix(')').unwrap_or(rest).trim().to_string();
            if let ExifTag::GPSLatitude(_) | ExifTag::GPSLongitude(_) = tag {
                value = format_gps(tag).unwrap_or(value);
            } else if value.starts_with('"') {
                value = value.trim_matches('"').trim_end_matches('\0').to_string();
            }
            if value.chars().count() > 160 {
                value = value.chars().take(160).collect::<String>() + "…";
            }
            Some(TagView {
                editable: EDITABLE.contains(&name),
                name: name.to_string(),
                group: format!("{:?}", tag.get_group()),
                value,
            })
        })
        .collect();
    tags.sort_by_key(|t| (t.group != "GENERIC", t.group.clone(), t.name.clone()));
    tags
}

fn format_gps(tag: &ExifTag) -> Option<String> {
    let (ExifTag::GPSLatitude(v) | ExifTag::GPSLongitude(v)) = tag else { return None };
    let part = |i: usize| v.get(i).map(|r| r.nominator as f64 / r.denominator.max(1) as f64);
    let decimal = part(0)? + part(1)? / 60.0 + part(2)? / 3600.0;
    Some(format!("{decimal:.6}°"))
}
