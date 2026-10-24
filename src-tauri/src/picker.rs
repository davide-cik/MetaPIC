//! Screen color picker: freezes every monitor into a full-screen overlay
//! window where the user clicks the pixel to sample.

use image::RgbaImage;
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, State, WebviewUrl, WebviewWindowBuilder};

const OVERLAY_PREFIX: &str = "picker-";

#[derive(Default)]
pub struct Captures(Mutex<Vec<RgbaImage>>);

struct Screen {
    x: i32,
    y: i32,
    image: RgbaImage,
}

fn capture_all() -> Result<Vec<Screen>, String> {
    let monitors = xcap::Monitor::all().map_err(|e| format!("impossibile elencare i monitor: {e}"))?;
    monitors
        .iter()
        .map(|m| {
            let image = m.capture_image().map_err(|e| format!("cattura dello schermo non riuscita: {e}"))?;
            Ok(Screen { x: m.x().unwrap_or(0), y: m.y().unwrap_or(0), image })
        })
        .collect()
}

/// Physical top-left corner of the monitor a capture belongs to. xcap reports
/// physical pixels on Windows but points on macOS, so match against Tauri's
/// monitor list, which is always physical.
fn physical_origin(app: &AppHandle, screen: &Screen) -> (i32, i32) {
    let monitors = app.available_monitors().unwrap_or_default();
    monitors
        .iter()
        .find(|m| {
            let p = m.position();
            let s = m.scale_factor();
            let near = |a: i32, b: f64| (a as f64 - b).abs() <= 2.0;
            (near(p.x, screen.x as f64) && near(p.y, screen.y as f64))
                || (near(p.x, screen.x as f64 * s) && near(p.y, screen.y as f64 * s))
        })
        .map(|m| (m.position().x, m.position().y))
        .unwrap_or((screen.x, screen.y))
}

#[tauri::command]
pub async fn start_color_pick(app: AppHandle, captures: State<'_, Captures>) -> Result<(), String> {
    let main = app.get_webview_window("main").ok_or("finestra principale non trovata")?;
    // Hide MetaPIC so it doesn't cover what the user wants to sample.
    main.hide().map_err(|e| e.to_string())?;
    let screens = tauri::async_runtime::spawn_blocking(|| {
        std::thread::sleep(Duration::from_millis(250));
        capture_all()
    })
    .await
    .map_err(|e| e.to_string())
    .and_then(|r| r);

    let screens = match screens {
        Ok(s) if !s.is_empty() => s,
        Ok(_) => {
            let _ = main.show();
            return Err("nessun monitor trovato".into());
        }
        Err(e) => {
            let _ = main.show();
            return Err(e);
        }
    };

    let mut overlays = Vec::new();
    for (i, screen) in screens.iter().enumerate() {
        let (x, y) = physical_origin(&app, screen);
        let window = WebviewWindowBuilder::new(&app, format!("{OVERLAY_PREFIX}{i}"), WebviewUrl::App("picker.html".into()))
            .title("MetaPIC – Preleva colore")
            .decorations(false)
            .resizable(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .shadow(false)
            .visible(false)
            .build()
            .map_err(|e| e.to_string())?;
        let _ = window.set_position(PhysicalPosition::new(x, y));
        let _ = window.set_size(PhysicalSize::new(screen.image.width(), screen.image.height()));
        overlays.push(window);
    }
    *captures.0.lock().unwrap() = screens.into_iter().map(|s| s.image).collect();
    for window in &overlays {
        let _ = window.show();
    }
    if let Some(first) = overlays.first() {
        let _ = first.set_focus();
    }
    Ok(())
}

/// Frozen pixels of one monitor: 8-byte header (width, height as u32 LE) followed by RGBA.
#[tauri::command]
pub fn capture_pixels(captures: State<'_, Captures>, index: usize) -> Result<tauri::ipc::Response, String> {
    let captures = captures.0.lock().unwrap();
    let image = captures.get(index).ok_or("cattura non disponibile")?;
    let mut bytes = Vec::with_capacity(8 + image.as_raw().len());
    bytes.extend_from_slice(&image.width().to_le_bytes());
    bytes.extend_from_slice(&image.height().to_le_bytes());
    bytes.extend_from_slice(image.as_raw());
    Ok(tauri::ipc::Response::new(bytes))
}

/// Closes every overlay and brings MetaPIC back, with the picked color if any.
#[tauri::command]
pub fn finish_color_pick(app: AppHandle, captures: State<'_, Captures>, color: Option<[u8; 3]>) {
    for (label, window) in app.webview_windows() {
        if label.starts_with(OVERLAY_PREFIX) {
            let _ = window.close();
        }
    }
    captures.0.lock().unwrap().clear();
    if let Some(main) = app.get_webview_window("main") {
        let _ = main.show();
        let _ = main.unminimize();
        let _ = main.set_focus();
    }
    if let Some(color) = color {
        let _ = app.emit_to("main", "color-picked", color);
    }
}

#[cfg(test)]
mod tests {
    /// Needs a real desktop session: `cargo test --lib -- --ignored captures_every_monitor`.
    #[test]
    #[ignore]
    fn captures_every_monitor() {
        let screens = super::capture_all().unwrap();
        assert!(!screens.is_empty());
        for s in &screens {
            println!("monitor @({}, {}): {}×{}", s.x, s.y, s.image.width(), s.image.height());
            assert!(s.image.width() > 0 && s.image.height() > 0);
        }
    }
}
