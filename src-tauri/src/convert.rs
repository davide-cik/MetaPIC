//! Single-file conversion pipeline: read → orient → resize → encode → metadata → write.

use crate::codec::{self, EncodeOptions, Format};
use crate::meta::{self, Edit, MetaMode};
use anyhow::{Context, Result};
use image::DynamicImage;
use image::imageops::FilterType;
use image::metadata::Orientation;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "mode", rename_all = "lowercase")]
pub enum Resize {
    None,
    /// Fit inside the box, never upscaling. 0 means unbounded on that side.
    Fit { width: u32, height: u32 },
    Percent { percent: f32 },
    Exact { width: u32, height: u32 },
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Options {
    pub format: Format,
    pub quality: u8,
    pub lossless: bool,
    pub speed: u8,
    pub resize: Resize,
    pub metadata: MetaMode,
    pub edits: Vec<Edit>,
    /// None writes next to the source file.
    pub output_dir: Option<PathBuf>,
    pub suffix: String,
    pub overwrite: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    pub output: PathBuf,
    pub input_bytes: u64,
    pub output_bytes: u64,
    pub width: u32,
    pub height: u32,
    pub millis: u64,
    pub notes: Vec<String>,
}

pub fn convert(src: &Path, opts: &Options) -> Result<Outcome> {
    let started = Instant::now();
    let mut notes = Vec::new();
    let bytes = std::fs::read(src).with_context(|| format!("impossibile leggere {}", src.display()))?;
    let (mut img, src_format) = codec::decode(&bytes)?;
    let source_meta = meta::read(&bytes, src_format);

    if let Some(o) = source_meta.as_ref().and_then(meta::orientation).and_then(Orientation::from_exif) {
        img.apply_orientation(o);
    }
    img = resize(img, &opts.resize);
    if opts.format == Format::Ico && (img.width() > 256 || img.height() > 256) {
        img = img.resize(256, 256, FilterType::Lanczos3);
        notes.push("ridotta a 256 px, il massimo per le icone".into());
    }

    let out_meta = meta::for_output(source_meta, opts.metadata, &opts.edits, img.width(), img.height());
    let exif_block = match (&out_meta, opts.format) {
        (Some(m), Format::Avif) => meta::tiff_block(m).ok(),
        _ => None,
    };

    let encode_opts = EncodeOptions { quality: opts.quality, lossless: opts.lossless, speed: opts.speed, exif: exif_block };
    let mut encoded = codec::encode(&img, opts.format, &encode_opts)?;
    if opts.format == Format::Jxl && !opts.lossless {
        notes.push("JPEG XL salvato lossless (encoder lossy non ancora disponibile)".into());
    }

    if let Some(m) = &out_meta {
        if !meta::writable(opts.format) {
            notes.push("metadati non supportati dal formato di destinazione".into());
        } else if opts.format != Format::Avif
            && let Err(e) = meta::embed(&mut encoded, opts.format, m) {
                notes.push(format!("metadati non scritti: {e}"));
            }
    }

    let output = output_path(src, opts)?;
    std::fs::write(&output, &encoded).with_context(|| format!("impossibile scrivere {}", output.display()))?;

    Ok(Outcome {
        output,
        input_bytes: bytes.len() as u64,
        output_bytes: encoded.len() as u64,
        width: img.width(),
        height: img.height(),
        millis: started.elapsed().as_millis() as u64,
        notes,
    })
}

fn resize(img: DynamicImage, resize: &Resize) -> DynamicImage {
    let (w, h) = (img.width(), img.height());
    match *resize {
        Resize::None => img,
        Resize::Fit { width, height } => {
            let max_w = if width == 0 { u32::MAX } else { width };
            let max_h = if height == 0 { u32::MAX } else { height };
            if w <= max_w && h <= max_h {
                img
            } else {
                img.resize(max_w.min(w), max_h.min(h), FilterType::Lanczos3)
            }
        }
        Resize::Percent { percent } => {
            let scale = (percent / 100.0).max(0.01);
            let nw = ((w as f32 * scale).round() as u32).max(1);
            let nh = ((h as f32 * scale).round() as u32).max(1);
            if (nw, nh) == (w, h) { img } else { img.resize_exact(nw, nh, FilterType::Lanczos3) }
        }
        Resize::Exact { width, height } if width > 0 && height > 0 => {
            img.resize_exact(width, height, FilterType::Lanczos3)
        }
        Resize::Exact { .. } => img,
    }
}

/// `<dir>/<stem><suffix>.<ext>`, numbered when it would clobber a file
/// (unless overwriting is allowed, and never the source itself).
fn output_path(src: &Path, opts: &Options) -> Result<PathBuf> {
    let dir = match &opts.output_dir {
        Some(d) => d.clone(),
        None => src.parent().map(Path::to_path_buf).unwrap_or_default(),
    };
    std::fs::create_dir_all(&dir).with_context(|| format!("impossibile creare {}", dir.display()))?;
    let stem = src.file_stem().unwrap_or_default().to_string_lossy();
    let ext = opts.format.extension();
    let candidate = |n: u32| {
        let counter = if n == 0 { String::new() } else { format!(" ({n})") };
        dir.join(format!("{stem}{}{counter}.{ext}", opts.suffix))
    };
    let is_source = |p: &Path| same_file(p, src);
    let first = candidate(0);
    if !first.exists() || (opts.overwrite && !is_source(&first)) {
        return Ok(first);
    }
    Ok((1..).map(candidate).find(|p| !p.exists()).unwrap())
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}
