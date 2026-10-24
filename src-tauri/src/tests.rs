//! End-to-end checks of the conversion pipeline on a generated photo.

use crate::codec::{self, Format};
use crate::convert::{self, Options, Resize};
use crate::meta::{self, Edit, MetaMode};
use image::{DynamicImage, RgbaImage};
use little_exif::exif_tag::ExifTag;
use little_exif::filetype::FileExtension;
use little_exif::metadata::Metadata;
use little_exif::rational::uR64;
use std::path::{Path, PathBuf};

const ALL_OUTPUTS: &[Format] = &[
    Format::Jpeg,
    Format::Png,
    Format::Webp,
    Format::Avif,
    Format::Jxl,
    Format::Tiff,
    Format::Gif,
    Format::Bmp,
    Format::Ico,
    Format::Qoi,
];

fn work_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("metapic-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A 300×200 JPEG with Artist, GPS and orientation 6 (rotate 90° clockwise).
fn sample_jpeg(dir: &Path) -> PathBuf {
    let img = RgbaImage::from_fn(300, 200, |x, y| image::Rgba([x as u8, y as u8, 128, 255]));
    let mut bytes = codec::encode(
        &DynamicImage::ImageRgba8(img),
        Format::Jpeg,
        &codec::EncodeOptions { quality: 90, lossless: false, speed: 10, exif: None },
    )
    .unwrap();
    let mut m = Metadata::new();
    m.set_tag(ExifTag::Artist("Mario Rossi".into()));
    m.set_tag(ExifTag::Orientation(vec![6]));
    let r = |n| uR64 { nominator: n, denominator: 1 };
    m.set_tag(ExifTag::GPSLatitudeRef("N".into()));
    m.set_tag(ExifTag::GPSLatitude(vec![r(45), r(30), r(0)]));
    m.write_to_vec(&mut bytes, FileExtension::JPEG).unwrap();
    let path = dir.join("foto.jpg");
    std::fs::write(&path, bytes).unwrap();
    path
}

fn options(format: Format, dir: &Path) -> Options {
    Options {
        format,
        quality: 70,
        lossless: false,
        speed: 10,
        resize: Resize::None,
        metadata: MetaMode::Keep,
        edits: vec![],
        output_dir: Some(dir.join("out")),
        suffix: String::new(),
        overwrite: false,
    }
}

fn tags_of(path: &Path) -> Vec<String> {
    let bytes = std::fs::read(path).unwrap();
    let format = codec::sniff(&bytes).unwrap();
    meta::read(&bytes, format).map(|m| meta::list(&m).into_iter().map(|t| format!("{}={}", t.name, t.value)).collect()).unwrap_or_default()
}

#[test]
fn converts_to_every_format_and_back() {
    let dir = work_dir("all");
    let src = sample_jpeg(&dir);
    for &format in ALL_OUTPUTS {
        let outcome = convert::convert(&src, &options(format, &dir)).unwrap_or_else(|e| panic!("{format:?}: {e:#}"));
        let bytes = std::fs::read(&outcome.output).unwrap();
        assert_eq!(codec::sniff(&bytes), Some(format), "{format:?} sniff");
        // Orientation 6 was applied: 300×200 becomes 200×300 (icons are capped at 256).
        let expected = if format == Format::Ico { (171, 256) } else { (200, 300) };
        assert_eq!((outcome.width, outcome.height), expected, "{format:?} dimensions");
        if format != Format::Avif || cfg!(feature = "avif-read") {
            let (img, _) = codec::decode(&bytes).unwrap_or_else(|e| panic!("{format:?} decode: {e:#}"));
            assert_eq!((img.width(), img.height()), expected, "{format:?} decoded dimensions");
        }
        println!("{format:?}: {} bytes {:?}", outcome.output_bytes, outcome.notes);

        if meta::writable(format) {
            let tags = tags_of(&outcome.output);
            assert!(tags.iter().any(|t| t == "Artist=Mario Rossi"), "{format:?} lost Artist: {tags:?}");
            assert!(tags.iter().any(|t| t == "Orientation=[1]"), "{format:?} orientation not reset: {tags:?}");
        }
    }
}

#[test]
fn metadata_modes_and_edits() {
    let dir = work_dir("meta");
    let src = sample_jpeg(&dir);

    let mut opts = options(Format::Webp, &dir);
    opts.metadata = MetaMode::KeepNoGps;
    opts.edits = vec![Edit { tag: "Copyright".into(), value: "© 2026".into() }];
    let tags = tags_of(&convert::convert(&src, &opts).unwrap().output);
    assert!(tags.iter().any(|t| t == "Artist=Mario Rossi"), "{tags:?}");
    assert!(tags.iter().any(|t| t == "Copyright=© 2026"), "{tags:?}");
    assert!(!tags.iter().any(|t| t.starts_with("GPS")), "GPS kept: {tags:?}");

    opts.metadata = MetaMode::Strip;
    opts.edits = vec![];
    assert!(tags_of(&convert::convert(&src, &opts).unwrap().output).is_empty());
}

#[test]
fn resize_and_naming() {
    let dir = work_dir("resize");
    let src = sample_jpeg(&dir);
    let mut opts = options(Format::Png, &dir);
    opts.resize = Resize::Fit { width: 100, height: 0 };
    opts.suffix = "-small".into();
    let first = convert::convert(&src, &opts).unwrap();
    assert_eq!((first.width, first.height), (100, 150));
    assert!(first.output.ends_with("foto-small.png"));
    let second = convert::convert(&src, &opts).unwrap();
    assert!(second.output.ends_with("foto-small (1).png"), "{:?}", second.output);

    opts.resize = Resize::Percent { percent: 50.0 };
    opts.overwrite = true;
    let third = convert::convert(&src, &opts).unwrap();
    assert_eq!((third.width, third.height), (100, 150));
    assert_eq!(third.output, first.output);
}

#[test]
fn edits_metadata_in_place() {
    let dir = work_dir("inplace");
    let src = sample_jpeg(&dir);
    for &format in &[Format::Jpeg, Format::Png, Format::Webp, Format::Tiff, Format::Jxl] {
        let converted = convert::convert(&src, &options(format, &dir)).unwrap().output;
        let bytes = std::fs::read(&converted).unwrap();
        let edits = [
            Edit { tag: "Artist".into(), value: "Luigi Bianchi".into() },
            Edit { tag: "ImageDescription".into(), value: "Lago di Como".into() },
        ];
        let edited = meta::rewrite(bytes, format, &edits, true, false).unwrap_or_else(|e| panic!("{format:?}: {e:#}"));
        std::fs::write(&converted, &edited).unwrap();
        let tags = tags_of(&converted);
        assert!(tags.iter().any(|t| t == "Artist=Luigi Bianchi"), "{format:?}: {tags:?}");
        assert!(tags.iter().any(|t| t == "ImageDescription=Lago di Como"), "{format:?}: {tags:?}");
        assert!(!tags.iter().any(|t| t.starts_with("GPS")), "{format:?} GPS kept: {tags:?}");
        if format != Format::Avif || cfg!(feature = "avif-read") {
            codec::decode(&edited).unwrap_or_else(|e| panic!("{format:?} corrupted: {e:#}"));
        }

        let stripped = meta::rewrite(edited, format, &[], false, true).unwrap_or_else(|e| panic!("{format:?} strip: {e:#}"));
        std::fs::write(&converted, &stripped).unwrap();
        // TIFF keeps its layout tags; nothing descriptive may survive.
        let left = tags_of(&converted);
        assert!(!left.iter().any(|t| t.starts_with("Artist") || t.starts_with("ImageDescription") || t.starts_with("GPS")), "{format:?} strip left {left:?}");
        if format != Format::Tiff {
            assert!(left.is_empty(), "{format:?} strip left {left:?}");
        }
        println!("{format:?}: in-place edit ok");
    }
}

#[test]
fn mozjpeg_beats_baseline_jpeg() {
    // Photo-like content: smooth gradients plus deterministic noise.
    let mut seed = 0x2545F491u32;
    let img = image::RgbImage::from_fn(800, 600, |x, y| {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        let n = (seed % 24) as i32 - 12;
        let c = |v: u32| (v as i32 + n).clamp(0, 255) as u8;
        image::Rgb([c(x * 255 / 800), c(y * 255 / 600), c((x + y) * 255 / 1400)])
    });
    let dynamic = DynamicImage::ImageRgb8(img.clone());
    for quality in [60u8, 75, 90] {
        let mut baseline = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut baseline, quality).encode_image(&img).unwrap();
        let opts = codec::EncodeOptions { quality, lossless: false, speed: 10, exif: None };
        let moz = codec::encode(&dynamic, Format::Jpeg, &opts).unwrap();
        let (decoded, _) = codec::decode(&moz).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (800, 600));
        let saving = 100.0 - moz.len() as f64 * 100.0 / baseline.len() as f64;
        println!("q{quality}: baseline {} B, mozjpeg {} B ({saving:.1}% in meno)", baseline.len(), moz.len());
        assert!(moz.len() < baseline.len(), "q{quality}: mozjpeg not smaller");
    }
}
