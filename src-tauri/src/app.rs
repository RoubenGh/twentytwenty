use crate::config::{MAX_PLAUSIBLE_IDLE_SECS, PAUSE_ONE_HOUR_MS, TICK_MS, WORK_INTERVAL_SECS};
use crate::engine::{Command as EngineCmd, Engine, TrayStatus, UserEvent};
use crate::probe::{self, ActivityProbe, Sample};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, Wry};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};

/// Name of the marker file, in the app's config directory, recording that the
/// first-run autostart question has already been asked. Its mere presence is
/// the whole state: contents are never read. Deliberately not a full
/// settings system -- this project has none by design, and one bit doesn't
/// need one.
const AUTOSTART_ASKED_FILE: &str = "autostart-asked";

pub struct AppState {
    pub engine: Mutex<Engine>,
}

/// Handle to the tray menu's first entry, a disabled item used purely as a
/// live readout ("Next break in 12:34"). Managed separately from `AppState`
/// because the menu does not exist until `build_tray` runs, which is after
/// the engine is managed in `lib.rs`.
pub struct TrayStatusLine(pub Mutex<MenuItem<Wry>>);

pub(crate) fn wall_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Runs the sensing loop forever. A probe error downgrades the rest of the
/// session to the fallback probe rather than killing the loop. An
/// implausibly large idle reading is NOT an error: it is clamped to
/// `MAX_PLAUSIBLE_IDLE_SECS` and the working probe is kept, because the
/// honest causes of one (a machine left idle over a weekend with sleep
/// disabled, a wrapped platform counter) are not evidence that the probe has
/// stopped working, and permanently downgrading to the input-idle-only
/// fallback would silently turn this app into the plain timer it exists not
/// to be. Every value above `ACTIVE_GRACE_SECS` is identical to the engine
/// anyway. Idle time falling back down is normal (the user just gave input)
/// and must never trigger a downgrade either.
pub fn spawn_loop(handle: AppHandle) {
    std::thread::spawn(move || {
        let mut probe: Box<dyn ActivityProbe> = probe::select();
        log::info!("using probe: {}", probe.name());
        let started = Instant::now();

        loop {
            std::thread::sleep(Duration::from_millis(TICK_MS));

            let sample = match probe.sample() {
                Ok(mut s) => {
                    if s.idle_seconds > MAX_PLAUSIBLE_IDLE_SECS {
                        log::warn!(
                            "probe reported implausible idle time ({}s), clamping to {}s",
                            s.idle_seconds,
                            MAX_PLAUSIBLE_IDLE_SECS
                        );
                        s.idle_seconds = s.idle_seconds.min(MAX_PLAUSIBLE_IDLE_SECS);
                    }
                    s
                }
                Err(e) => {
                    log::warn!("probe failed, downgrading to fallback: {e}");
                    probe = Box::new(crate::probe::fallback::FallbackProbe::new());
                    Sample::default()
                }
            };

            let mono = started.elapsed().as_millis() as u64;
            let cmds = {
                let state = handle.state::<AppState>();
                let mut engine = state.engine.lock().unwrap();
                engine.tick(sample, mono, wall_ms())
            };
            dispatch(&handle, cmds);
        }
    });
}

pub fn dispatch(handle: &AppHandle, cmds: Vec<EngineCmd>) {
    for cmd in cmds {
        match cmd {
            EngineCmd::ShowOverlay => crate::overlay::show(handle),
            EngineCmd::HideOverlay => crate::overlay::hide(handle),
            EngineCmd::UpdateOverlay { remaining_secs } => {
                crate::overlay::update(handle, remaining_secs)
            }
            EngineCmd::Notify { title, body } => crate::overlay::notify(handle, &title, &body),
            EngineCmd::TrayStatus(status) => update_tray(handle, status),
        }
    }
}

/// Text for the tray's live readout. Working shows a real countdown in
/// m:ss so the menu tells you exactly how long you have, not a rounded
/// "12 min" that sits unchanged for a minute at a time.
fn status_text(status: TrayStatus) -> String {
    match status {
        TrayStatus::Working { bank_secs } => {
            let left = WORK_INTERVAL_SECS.saturating_sub(bank_secs);
            if left == 0 {
                "Next break: now".to_string()
            } else {
                format!("Next break in {}:{:02}", left / 60, left % 60)
            }
        }
        TrayStatus::Break => "Break in progress, look away".to_string(),
        TrayStatus::Snoozed => "Snoozed, break coming back".to_string(),
        TrayStatus::Paused => "Paused".to_string(),
    }
}

fn update_tray(handle: &AppHandle, status: TrayStatus) {
    let tip = match status {
        TrayStatus::Working { bank_secs } => {
            let left = WORK_INTERVAL_SECS.saturating_sub(bank_secs);
            // Ceiling division: a full 1200-second interval must read as 20
            // minutes, not 21 (`left / 60 + 1` over-reported by one minute
            // whenever `left` was an exact multiple of 60).
            format!(
                "TwentyTwenty: {} min until your next break",
                (left + 59) / 60
            )
        }
        TrayStatus::Break => "TwentyTwenty: look away".to_string(),
        TrayStatus::Snoozed => "TwentyTwenty: snoozed".to_string(),
        TrayStatus::Paused => "TwentyTwenty: paused".to_string(),
    };
    if let Some(tray) = handle.tray_by_id("main") {
        let _ = tray.set_tooltip(Some(&tip));
    }

    // The menu readout. Hover gives you the tooltip; right-clicking gives you
    // this, which is the discoverable one. Failure here is never fatal: a
    // stale countdown is not worth taking the app down for.
    if let Some(line) = handle.try_state::<TrayStatusLine>() {
        match line.0.lock() {
            Ok(item) => {
                if let Err(e) = item.set_text(status_text(status)) {
                    log::debug!("could not update tray status line: {e}");
                }
            }
            Err(e) => log::debug!("tray status line mutex poisoned: {e}"),
        }
    }
}

/// Asks, once ever, whether the user wants TwentyTwenty to start
/// automatically at login. The "already asked" bit lives in a marker file in
/// the app's config directory (never in the repo, never a full settings
/// file); a missing or unreadable marker is treated as "not asked yet" so a
/// one-time filesystem hiccup can't turn into a permanent silent skip in one
/// direction, and any failure anywhere in this path (resolving the config
/// dir, showing the dialog, writing the marker, registering autostart) is
/// logged and swallowed -- this must never be a reason the app fails to
/// start. Autostart can still be flipped later from the tray menu (see
/// `build_tray`), so a "No" here is not permanent.
pub fn maybe_ask_autostart(app: &tauri::App) {
    let marker = match app.path().app_config_dir() {
        Ok(dir) => dir.join(AUTOSTART_ASKED_FILE),
        Err(e) => {
            log::warn!("could not resolve app config dir, skipping autostart prompt: {e}");
            return;
        }
    };

    if marker.exists() {
        return;
    }

    let handle = app.handle().clone();
    app.dialog()
        .message("Start TwentyTwenty automatically when you log in?")
        .title("TwentyTwenty")
        .buttons(MessageDialogButtons::YesNo)
        .show(move |yes| {
            if let Some(parent) = marker.parent() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    log::warn!("could not create app config dir: {e}");
                }
            }
            if let Err(e) = std::fs::write(&marker, b"") {
                log::warn!("could not persist autostart-asked marker: {e}");
            }

            if yes {
                if let Err(e) = handle.autolaunch().enable() {
                    log::warn!("could not enable autostart: {e}");
                }
            }
        });
}

pub fn build_tray(app: &tauri::App) -> tauri::Result<()> {
    // A disabled first entry used purely as a live readout. Disabled so it
    // cannot be clicked or focused; its text is rewritten every tick by
    // `update_tray` from the same TrayStatus the tooltip uses.
    let status_line = MenuItem::with_id(
        app,
        "status_line",
        status_text(TrayStatus::Working { bank_secs: 0 }),
        false,
        None::<&str>,
    )?;
    let break_now = MenuItem::with_id(app, "break_now", "Take a break now", true, None::<&str>)?;
    let snooze = MenuItem::with_id(app, "snooze", "Snooze 5 minutes", true, None::<&str>)?;
    let pause_hour = MenuItem::with_id(app, "pause_hour", "Pause for 1 hour", true, None::<&str>)?;
    let pause = MenuItem::with_id(app, "pause", "Pause until I resume", true, None::<&str>)?;
    let resume = MenuItem::with_id(app, "resume", "Resume", true, None::<&str>)?;
    let autostart_enabled = app.autolaunch().is_enabled().unwrap_or_else(|e| {
        log::warn!("could not read autostart state: {e}");
        false
    });
    let autostart_item = CheckMenuItem::with_id(
        app,
        "autostart",
        "Start at login",
        true,
        autostart_enabled,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &status_line,
            &PredefinedMenuItem::separator(app)?,
            &break_now,
            &snooze,
            &pause_hour,
            &pause,
            &resume,
            &autostart_item,
            &quit,
        ],
    )?;

    app.manage(TrayStatusLine(Mutex::new(status_line)));

    // `default_window_icon()` is generated at build time from `tauri.conf.json`'s
    // `bundle.icon` list (falling back to `icons/icon.png`/`icons/icon.ico` on
    // disk); it does not depend on `app.windows` being non-empty (that key
    // controls startup *windows*, not the icon bundle). Verified against the
    // tauri-codegen source (`context.rs`): the icon lookup runs unconditionally,
    // so in this repo's config it always returns `Some`. Even so, never unwrap
    // something that is typed `Option` at a startup path.
    //
    // If it were ever `None` (a broken build, a missing icon file), this is NOT
    // a case to degrade gracefully into: this app has no window (`app.windows:
    // []`) and no other UI, so a tray-less run is not a degraded app, it is an
    // invisible, unkillable one -- `lib.rs`'s `run()` unconditionally swallows
    // the exit that would otherwise fire when the (nonexistent) tray's Quit
    // item can't be clicked. Exit loudly here instead, before the event loop
    // (and that swallow) ever starts, so the two degradations can't compose.
    let icon = match app.default_window_icon().cloned() {
        Some(icon) => icon,
        None => {
            log::error!(
                "no default window icon available; a tray-less TwentyTwenty would be an \
                 invisible process with no way to quit it, so exiting instead of starting"
            );
            std::process::exit(1);
        }
    };

    TrayIconBuilder::with_id("main")
        .icon(icon)
        .tooltip("TwentyTwenty")
        .menu(&menu)
        .on_menu_event(move |handle, event| {
            let ev = match event.id().as_ref() {
                "break_now" => Some(UserEvent::BreakNow),
                "snooze" => Some(UserEvent::Snooze),
                "pause_hour" => Some(UserEvent::Pause {
                    for_ms: Some(PAUSE_ONE_HOUR_MS),
                }),
                "pause" => Some(UserEvent::Pause { for_ms: None }),
                "resume" => Some(UserEvent::Resume),
                "autostart" => {
                    // Ground truth is always the autostart plugin, not the
                    // checkbox: toggle it, then set the checkbox to whatever
                    // the plugin now actually reports, so a failed
                    // enable/disable can't leave the tray showing a state
                    // that isn't real.
                    let autostart = handle.autolaunch();
                    let currently_enabled = autostart.is_enabled().unwrap_or(false);
                    let result = if currently_enabled {
                        autostart.disable()
                    } else {
                        autostart.enable()
                    };
                    if let Err(e) = result {
                        log::warn!("could not toggle autostart: {e}");
                    }
                    let now_enabled = autostart.is_enabled().unwrap_or(currently_enabled);
                    if let Err(e) = autostart_item.set_checked(now_enabled) {
                        log::warn!("could not update autostart menu item: {e}");
                    }
                    None
                }
                "quit" => {
                    handle.exit(0);
                    None
                }
                _ => None,
            };
            if let Some(ev) = ev {
                // This closure runs on the main event-loop thread, and
                // `dispatch` reaches `WebviewWindowBuilder::build()` (via
                // `overlay::show`), which Tauri's own doc comment says
                // deadlocks on Windows when called on the main thread from
                // inside an event handler. A deadlock here is unrecoverable
                // for the user: the tray stops responding and the app has no
                // window, so the only way out is Task Manager. Do the work on
                // a worker thread instead, exactly as the sensing loop does.
                //
                // The engine `MutexGuard` is still confined to its own inner
                // block and dropped before `dispatch`, so the lock is never
                // held across overlay/tray work.
                let handle = handle.clone();
                std::thread::spawn(move || {
                    let cmds = {
                        let state = handle.state::<AppState>();
                        let mut engine = state.engine.lock().unwrap();
                        engine.on_user(ev, wall_ms())
                    };
                    dispatch(&handle, cmds);
                });
            }
        })
        .build(app)?;
    Ok(())
}
