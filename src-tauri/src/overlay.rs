//! The break overlay: one borderless, always-on-top, transparent, fullscreen
//! window per monitor. See `docs/findings/2026-09-10-wayland-overlay.md` for
//! why this shape (`FULLSCREEN_PER_MONITOR`) was chosen over a single
//! centered window, and why the overlay windows need their own Tauri
//! capability (`src-tauri/capabilities/overlay.json`) to emit/listen/close.
//!
//! # The window is created INERT and earns the right to capture input
//!
//! Nothing here becomes always-on-top or focused, and nothing stops accepting
//! clicks, until the page inside it has emitted `tt://overlay-ready`, which it
//! only does after it has actually painted a frame. Until then the window is
//! click-through (`ignore_cursor_events`), unfocused and not always-on-top, so
//! it captures neither the pointer nor the keyboard.
//!
//! It is deliberately NOT created hidden, which was the obvious first attempt
//! and is a deadlock: a window that is not mapped never composites a frame, so
//! `requestAnimationFrame` never fires, so the page can never report that it
//! painted, so a perfectly healthy overlay is abandoned every time. The window
//! must be on screen to prove itself. Making it *inert* rather than *hidden*
//! is what squares that circle.
//!
//! That ordering is the whole point, not a nicety. This window swallows every
//! click and keypress by design, and *every* affordance that dismisses it --
//! Escape, Snooze, Skip -- lives in the page's script. So a window that is
//! shown before its content renders is not a degraded overlay, it is a total
//! input trap: nothing on screen to explain itself, and no way out short of
//! killing the process from another TTY. That shipped, and a user lost their
//! session to it twice. See `docs/findings/2026-09-11-overlay-input-trap.md`.
//!
//! Two real failures are covered by this one rule:
//!   - the page cannot load at all (a binary built without
//!     `tauri/custom-protocol` resolves it against `build.devUrl`), and
//!   - the page loads but WebKit cannot render (the AppImage runtime bundles
//!     its own EGL libraries; against a host driver that disagrees the web
//!     process dies with `EGL_BAD_PARAMETER` before running a line of JS).
//!
//! Neither can be detected by asking Tauri whether the window was built --
//! it was, successfully, in both cases. The only trustworthy signal is the
//! page itself reporting that it drew something, so that is what we wait for.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_notification::NotificationExt;

use crate::config::OVERLAY_READY_TIMEOUT_MS;

const PREFIX: &str = "tt-overlay-";

/// Whether the current break's overlay has reported that it painted.
/// Reset by `show`, set by `mark_ready`, read by the watchdog.
static READY: AtomicBool = AtomicBool::new(false);

pub fn show(handle: &AppHandle) {
    let monitors = match handle.available_monitors() {
        Ok(m) => m,
        Err(e) => {
            log::error!("cannot enumerate monitors: {e}");
            fallback_notify(handle);
            return;
        }
    };
    if monitors.is_empty() {
        log::error!("no monitors reported; cannot place an overlay");
        fallback_notify(handle);
        return;
    }

    READY.store(false, Ordering::SeqCst);
    let mut built_any = false;

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
            // Inert until the page proves it rendered: click-through, no
            // focus, not above anything. `mark_ready` lifts all three the
            // moment `tt://overlay-ready` arrives. An overlay that never
            // renders therefore stays a fully transparent, fully
            // pass-through window that the user can click and type straight
            // through, rather than the input trap that shipped twice.
            .focused(false)
            .always_on_top(false)
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
            .position(pos.x as f64, pos.y as f64)
            .inner_size(size.width as f64, size.height as f64)
            .build();

        match built {
            Ok(w) => {
                // Not a builder option in tauri 2.11, so applied immediately
                // after construction. This is the single most important line
                // in the file: it is what makes an unrendered overlay
                // harmless instead of a screen lock.
                let _ = w.set_ignore_cursor_events(true);
                built_any = true;
            }
            Err(e) => log::error!("cannot build overlay on monitor {i}: {e}"),
        }
    }

    if !built_any {
        fallback_notify(handle);
        return;
    }

    // Watchdog. If the page never reports a painted frame, tear the windows
    // down and degrade to a notification rather than leaving hidden windows
    // around for a break that will never be visible.
    let h = handle.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(OVERLAY_READY_TIMEOUT_MS));
        if READY.load(Ordering::SeqCst) {
            return;
        }
        log::error!(
            "overlay did not report a rendered frame within {OVERLAY_READY_TIMEOUT_MS}ms; \
             abandoning it and notifying instead"
        );
        hide(&h);
        // Deliberately says WHY, not just "time for a break". A user whose
        // overlay silently stopped appearing has no way to tell a broken
        // renderer from a broken app, and the actionable answer (this build
        // cannot draw on this machine, use a native package) is not something
        // they could ever guess.
        notify(
            &h,
            "Time to rest your eyes",
            "Look 20 feet away for 20 seconds. (The full-screen reminder could \
             not be drawn on this system, so this is a notification instead.)",
        );
    });
}

/// Called when an overlay window reports that it has painted. Promotes every
/// overlay window from inert to the real thing: click-capturing, fullscreen,
/// always-on-top, and (for the first one) focused so Escape reaches it.
pub fn mark_ready(handle: &AppHandle) {
    // The page emits once per window; only the first report does the work.
    if READY.swap(true, Ordering::SeqCst) {
        return;
    }
    let mut first = true;
    for (label, window) in handle.webview_windows() {
        if !label.starts_with(PREFIX) {
            continue;
        }
        let _ = window.set_ignore_cursor_events(false);
        let _ = window.set_always_on_top(true);
        let _ = window.set_fullscreen(true);
        if first {
            let _ = window.set_focus();
            first = false;
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

/// What a break degrades to when no overlay can be put on screen.
fn fallback_notify(handle: &AppHandle) {
    notify(
        handle,
        "Time to rest your eyes",
        "Look at something 20 feet away for 20 seconds.",
    );
}
