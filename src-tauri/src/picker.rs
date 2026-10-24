//! Live screen color picker. A transparent overlay window covers each monitor
//! to catch the mouse; the overlays are excluded from screen capture, so the
//! pixels read under the cursor are always those of the real desktop.

use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

const OVERLAY_PREFIX: &str = "picker-";
/// Side of the square read around the cursor (odd, so there is a center pixel).
const SAMPLE: u32 = 11;

#[tauri::command]
pub async fn start_color_pick(app: AppHandle) -> Result<(), String> {
    let main = app.get_webview_window("main").ok_or("finestra principale non trovata")?;
    let monitors = app.available_monitors().map_err(|e| e.to_string())?;
    if monitors.is_empty() {
        return Err("nessun monitor trovato".into());
    }
    // Get MetaPIC out of the way so everything behind it can be sampled.
    main.hide().map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(|| std::thread::sleep(Duration::from_millis(150)))
        .await
        .map_err(|e| e.to_string())?;

    let mut overlays = Vec::new();
    for (i, monitor) in monitors.iter().enumerate() {
        let built = WebviewWindowBuilder::new(&app, format!("{OVERLAY_PREFIX}{i}"), WebviewUrl::App("picker.html".into()))
            .title("MetaPIC – Preleva colore")
            .decorations(false)
            .transparent(true)
            .resizable(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .shadow(false)
            // Keeps the overlay out of screen capture (WDA_EXCLUDEFROMCAPTURE on Windows).
            .content_protected(true)
            .visible(false)
            .build();
        let window = match built {
            Ok(w) => w,
            Err(e) => {
                close_overlays(&app);
                let _ = main.show();
                return Err(format!("impossibile aprire il selettore: {e}"));
            }
        };
        let _ = window.set_position(*monitor.position());
        let _ = window.set_size(*monitor.size());
        overlays.push(window);
    }
    for window in &overlays {
        let _ = window.show();
    }
    if let Some(first) = overlays.first() {
        let _ = first.set_focus();
    }
    Ok(())
}

/// Reads the pixels around the mouse cursor, wherever it is.
/// Response: 16-byte header (cursor column, cursor row, width, height as u32 LE) + RGBA.
#[tauri::command]
pub fn sample_cursor(app: AppHandle) -> Result<tauri::ipc::Response, String> {
    let cursor = app.cursor_position().map_err(|e| e.to_string())?;
    let (x, y) = (cursor.x.floor() as i32, cursor.y.floor() as i32);
    let (column, row, image) = sample_at(x, y)?;
    let mut bytes = Vec::with_capacity(16 + image.as_raw().len());
    for v in [column, row, image.width(), image.height()] {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    bytes.extend_from_slice(image.as_raw());
    Ok(tauri::ipc::Response::new(bytes))
}

/// Captures a SAMPLE×SAMPLE square around a physical screen point, shifted
/// inward at monitor edges. Returns the point's position inside the square.
fn sample_at(x: i32, y: i32) -> Result<(u32, u32, image::RgbaImage), String> {
    // Match against the monitors' physical rectangles rather than MonitorFromPoint,
    // whose coordinates depend on the calling thread's DPI awareness.
    let geometry = |m: &xcap::Monitor| Some((m.x().ok()?, m.y().ok()?, m.width().ok()?, m.height().ok()?));
    let (monitor, (mx, my, mw, mh)) = xcap::Monitor::all()
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter_map(|m| geometry(&m).map(|g| (m, g)))
        .find(|(_, (mx, my, mw, mh))| x >= *mx && y >= *my && x < mx + *mw as i32 && y < my + *mh as i32)
        .ok_or("il puntatore non è su nessun monitor")?;
    let (w, h) = (SAMPLE.min(mw), SAMPLE.min(mh));
    let half = SAMPLE as i32 / 2;
    let rel_x = (x - mx).clamp(0, mw as i32 - 1);
    let rel_y = (y - my).clamp(0, mh as i32 - 1);
    let left = (rel_x - half).clamp(0, (mw - w) as i32);
    let top = (rel_y - half).clamp(0, (mh - h) as i32);
    let image = monitor
        .capture_region(left as u32, top as u32, w, h)
        .map_err(|e| format!("lettura dello schermo non riuscita: {e}"))?;
    Ok(((rel_x - left) as u32, (rel_y - top) as u32, image))
}

/// Moves the mouse by whole physical pixels, for precise picking with the arrow keys.
#[tauri::command]
pub fn nudge_cursor(app: AppHandle, window: WebviewWindow, dx: i32, dy: i32) -> Result<(), String> {
    let cursor = app.cursor_position().map_err(|e| e.to_string())?;
    let origin = window.inner_position().map_err(|e| e.to_string())?;
    let target = PhysicalPosition::new(cursor.x.floor() as i32 + dx - origin.x, cursor.y.floor() as i32 + dy - origin.y);
    window.set_cursor_position(target).map_err(|e| e.to_string())
}

fn close_overlays(app: &AppHandle) {
    for (label, window) in app.webview_windows() {
        if label.starts_with(OVERLAY_PREFIX) {
            let _ = window.close();
        }
    }
}

/// Closes every overlay and brings MetaPIC back, with the picked color if any.
#[tauri::command]
pub fn finish_color_pick(app: AppHandle, color: Option<[u8; 3]>) {
    close_overlays(&app);
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
    /// Needs a real desktop session: `cargo test --lib -- --ignored samples_every_monitor`.
    #[test]
    #[ignore]
    fn samples_every_monitor() {
        // The app is per-monitor DPI aware (tao sets it at startup); do the same here.
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext};
            SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        }
        for m in xcap::Monitor::all().unwrap() {
            let (x, y) = (m.x().unwrap(), m.y().unwrap());
            let (w, h) = (m.width().unwrap() as i32, m.height().unwrap() as i32);
            // Center, top-left corner and bottom-right corner of each monitor.
            for (px, py) in [(x + w / 2, y + h / 2), (x, y), (x + w - 1, y + h - 1)] {
                let (col, row, img) = super::sample_at(px, py).unwrap();
                assert_eq!((img.width(), img.height()), (super::SAMPLE, super::SAMPLE));
                assert!(col < super::SAMPLE && row < super::SAMPLE);
                println!("({px}, {py}) -> {:?} at ({col}, {row})", img.get_pixel(col, row).0);
            }
        }
    }
}
