//! The break overlay: one borderless, always-on-top, transparent, fullscreen
//! window per monitor. See `docs/findings/2026-09-10-wayland-overlay.md` for
//! why this shape (`FULLSCREEN_PER_MONITOR`) was chosen over a single
//! centered window, and why the overlay windows need their own Tauri
//! capability (`src-tauri/capabilities/overlay.json`) to emit/listen/close.

use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_notification::NotificationExt;

const PREFIX: &str = "tt-overlay-";

pub fn show(handle: &AppHandle) {
    let monitors = match handle.available_monitors() {
        Ok(m) => m,
        Err(e) => {
            log::error!("cannot enumerate monitors: {e}");
            return;
        }
    };
    for (i, m) in monitors.iter().enumerate() {
        let label = format!("{PREFIX}{i}");
        if handle.get_webview_window(&label).is_some() {
            continue;
        }
        let pos = m.position();
        let size = m.size();
        let built = WebviewWindowBuilder::new(handle, &label, WebviewUrl::App("overlay.html".into()))
            .title("TwentyTwenty")
            .decorations(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .transparent(true)
            .focused(i == 0)
            .position(pos.x as f64, pos.y as f64)
            .inner_size(size.width as f64, size.height as f64)
            .build();

        match built {
            Ok(w) => {
                let _ = w.set_fullscreen(true);
            }
            Err(e) => log::error!("cannot build overlay on monitor {i}: {e}"),
        }
    }
}

pub fn hide(handle: &AppHandle) {
    for (label, window) in handle.webview_windows() {
        if label.starts_with(PREFIX) {
            let _ = window.close();
        }
    }
}

pub fn update(handle: &AppHandle, remaining_secs: u64) {
    let _ = handle.emit("tt://tick", remaining_secs);
}

pub fn notify(handle: &AppHandle, title: &str, body: &str) {
    let _ = handle.notification().builder().title(title).body(body).show();
}
