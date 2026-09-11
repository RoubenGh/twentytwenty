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
            // KNOWN PLATFORM LIMITATION, confirmed by a user report on this
            // exact machine (KDE Plasma 6.7.4, Wayland): the overlay still
            // shows up as a taskbar entry despite this. Traced into `tao`
            // (0.35.3, `platform_impl/linux/window.rs` and `event_loop.rs`):
            // on Linux this hint is implemented as GTK's
            // `set_skip_taskbar_hint`/`set_skip_pager_hint`, which are pure
            // X11/EWMH (`_NET_WM_STATE_SKIP_TASKBAR`) calls under the hood --
            // GTK's Wayland backend has no equivalent and silently no-ops
            // them. Wayland's core protocol has no client-side "hide me from
            // the taskbar" hint at all; that is deliberately the
            // compositor's/shell's call, not a client's. Not fixable from
            // here; tracked on the Task 16 smoke checklist as a known
            // Wayland limitation rather than a bug to chase.
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
