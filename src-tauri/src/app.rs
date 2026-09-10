use crate::config::{MAX_PLAUSIBLE_IDLE_SECS, PAUSE_ONE_HOUR_MS, TICK_MS, WORK_INTERVAL_SECS};
use crate::engine::{Command as EngineCmd, Engine, TrayStatus, UserEvent};
use crate::probe::{self, ActivityProbe, Sample};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager};

pub struct AppState {
    pub engine: Mutex<Engine>,
}

fn wall_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Runs the sensing loop forever. A probe error, or an implausible sample
/// (more idle time than a real desktop session could produce), downgrades
/// the rest of the session to the fallback probe rather than killing the
/// loop. Idle time falling back down is normal (the user just gave input)
/// and must never trigger a downgrade.
pub fn spawn_loop(handle: AppHandle) {
    std::thread::spawn(move || {
        let mut probe: Box<dyn ActivityProbe> = probe::select();
        log::info!("using probe: {}", probe.name());
        let started = Instant::now();

        loop {
            std::thread::sleep(Duration::from_millis(TICK_MS));

            let sample = match probe.sample() {
                Ok(s) if s.idle_seconds > MAX_PLAUSIBLE_IDLE_SECS => {
                    log::warn!(
                        "probe reported implausible idle time ({}s), downgrading to fallback",
                        s.idle_seconds
                    );
                    probe = Box::new(crate::probe::fallback::FallbackProbe::new());
                    Sample::default()
                }
                Ok(s) => s,
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

fn update_tray(handle: &AppHandle, status: TrayStatus) {
    let tip = match status {
        TrayStatus::Working { bank_secs } => {
            let left = WORK_INTERVAL_SECS.saturating_sub(bank_secs);
            format!("TwentyTwenty: {} min until your next break", left / 60 + 1)
        }
        TrayStatus::Break => "TwentyTwenty: look away".to_string(),
        TrayStatus::Snoozed => "TwentyTwenty: snoozed".to_string(),
        TrayStatus::Paused => "TwentyTwenty: paused".to_string(),
    };
    if let Some(tray) = handle.tray_by_id("main") {
        let _ = tray.set_tooltip(Some(&tip));
    }
}

pub fn build_tray(app: &tauri::App) -> tauri::Result<()> {
    let break_now = MenuItem::with_id(app, "break_now", "Take a break now", true, None::<&str>)?;
    let snooze = MenuItem::with_id(app, "snooze", "Snooze 5 minutes", true, None::<&str>)?;
    let pause_hour = MenuItem::with_id(app, "pause_hour", "Pause for 1 hour", true, None::<&str>)?;
    let pause = MenuItem::with_id(app, "pause", "Pause until I resume", true, None::<&str>)?;
    let resume = MenuItem::with_id(app, "resume", "Resume", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[&break_now, &snooze, &pause_hour, &pause, &resume, &quit],
    )?;

    // `default_window_icon()` is generated at build time from `tauri.conf.json`'s
    // `bundle.icon` list (falling back to `icons/icon.png`/`icons/icon.ico` on
    // disk); it does not depend on `app.windows` being non-empty (that key
    // controls startup *windows*, not the icon bundle). Verified against the
    // tauri-codegen source (`context.rs`): the icon lookup runs unconditionally.
    // Even so, never unwrap something that is typed `Option` at a startup path:
    // if it is ever `None` (a broken build, a missing icon file), log loudly and
    // skip building the tray rather than panicking the whole app before it can
    // do anything.
    let icon = match app.default_window_icon().cloned() {
        Some(icon) => icon,
        None => {
            log::error!("no default window icon available; refusing to unwrap, tray will be missing");
            return Ok(());
        }
    };

    TrayIconBuilder::with_id("main")
        .icon(icon)
        .tooltip("TwentyTwenty")
        .menu(&menu)
        .on_menu_event(|handle, event| {
            let ev = match event.id().as_ref() {
                "break_now" => Some(UserEvent::BreakNow),
                "snooze" => Some(UserEvent::Snooze),
                "pause_hour" => Some(UserEvent::Pause {
                    for_ms: Some(PAUSE_ONE_HOUR_MS),
                }),
                "pause" => Some(UserEvent::Pause { for_ms: None }),
                "resume" => Some(UserEvent::Resume),
                "quit" => {
                    handle.exit(0);
                    None
                }
                _ => None,
            };
            if let Some(ev) = ev {
                let cmds = {
                    let state = handle.state::<AppState>();
                    let mut engine = state.engine.lock().unwrap();
                    engine.on_user(ev, wall_ms())
                };
                dispatch(handle, cmds);
            }
        })
        .build(app)?;
    Ok(())
}
