mod codec;
mod convert;
mod heif;
mod meta;
mod picker;
#[cfg(test)]
mod tests;

use base64::Engine;
use rayon::prelude::*;
use serde::Serialize;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Emitter, Manager, State};

#[derive(Default)]
struct Cancel(AtomicBool);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FileEntry {
    path: PathBuf,
    name: String,
    size: u64,
    format: codec::Format,
}

fn sniff_file(path: &Path) -> Option<codec::Format> {
    let mut head = [0u8; 64];
    let n = std::fs::File::open(path).ok()?.read(&mut head).ok()?;
    codec::sniff(&head[..n])
}

fn collect(path: &Path, out: &mut Vec<FileEntry>) {
    if path.is_dir() {
        let Ok(entries) = std::fs::read_dir(path) else { return };
        let mut children: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        children.sort();
        for child in children {
            collect(&child, out);
        }
    } else if let Some(format) = sniff_file(path) {
        out.push(FileEntry {
            name: path.file_name().unwrap_or_default().to_string_lossy().into_owned(),
            size: path.metadata().map(|m| m.len()).unwrap_or(0),
            path: path.to_path_buf(),
            format,
        });
    }
}

/// Expands dropped files and folders (recursively) into recognised images.
#[tauri::command]
fn expand_paths(paths: Vec<PathBuf>) -> Vec<FileEntry> {
    let mut out = Vec::new();
    for p in paths {
        collect(&p, &mut out);
    }
    out
}

/// Small JPEG preview as a data URL, so the UI can show formats the webview can't (JXL…).
#[tauri::command]
async fn thumbnail(path: PathBuf) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
        let (img, _) = codec::decode(&bytes).map_err(|e| e.to_string())?;
        let thumb = img.thumbnail(160, 160).to_rgb8();
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 80)
            .encode_image(&thumb)
            .map_err(|e| e.to_string())?;
        Ok(format!("data:image/jpeg;base64,{}", base64::engine::general_purpose::STANDARD.encode(jpeg)))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Progress {
    path: PathBuf,
    outcome: Option<convert::Outcome>,
    error: Option<String>,
}

/// Converts all files in parallel, emitting a `progress` event per file.
#[tauri::command]
async fn convert_files(
    app: AppHandle,
    cancel: State<'_, Cancel>,
    paths: Vec<PathBuf>,
    options: convert::Options,
) -> Result<(), String> {
    cancel.0.store(false, Ordering::SeqCst);
    tauri::async_runtime::spawn_blocking(move || {
        let cancel = app.state::<Cancel>();
        paths.par_iter().for_each(|path| {
            let result = if cancel.0.load(Ordering::SeqCst) {
                Err("annullato".to_string())
            } else {
                convert::convert(path, &options).map_err(|e| format!("{e:#}"))
            };
            let (outcome, error) = match result {
                Ok(o) => (Some(o), None),
                Err(e) => (None, Some(e)),
            };
            let _ = app.emit("progress", Progress { path: path.clone(), outcome, error });
        });
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn cancel_conversion(cancel: State<'_, Cancel>) {
    cancel.0.store(true, Ordering::SeqCst);
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MetadataView {
    format: codec::Format,
    writable: bool,
    tags: Vec<meta::TagView>,
    editable: Vec<&'static str>,
}

#[tauri::command]
fn read_metadata(path: PathBuf) -> Result<MetadataView, String> {
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    let format = codec::sniff(&bytes).ok_or("formato non riconosciuto")?;
    let tags = meta::read(&bytes, format).map(|m| meta::list(&m)).unwrap_or_default();
    Ok(MetadataView { format, writable: meta::writable_in_place(format), tags, editable: meta::EDITABLE.to_vec() })
}

/// Edits metadata in place, without re-encoding the pixels.
#[tauri::command]
fn write_metadata(path: PathBuf, edits: Vec<meta::Edit>, remove_gps: bool, strip_all: bool) -> Result<(), String> {
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    let format = codec::sniff(&bytes).ok_or("formato non riconosciuto")?;
    let bytes = meta::rewrite(bytes, format, &edits, remove_gps, strip_all).map_err(|e| format!("{e:#}"))?;
    // Write beside the original first so a failure never leaves a truncated file.
    let tmp = path.with_extension("metapic-tmp");
    std::fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(Cancel::default())
        .invoke_handler(tauri::generate_handler![
            expand_paths,
            thumbnail,
            convert_files,
            cancel_conversion,
            read_metadata,
            write_metadata,
            picker::start_color_pick,
            picker::sample_cursor,
            picker::nudge_cursor,
            picker::finish_color_pick
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
