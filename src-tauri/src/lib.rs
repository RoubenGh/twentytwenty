pub mod app;
pub mod config;
pub mod engine;
pub mod overlay;
pub mod probe;

use tauri::{Listener, Manager};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::init();
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|_app, _argv, _cwd| {}))
        .plugin(tauri_plugin_notification::init())
        .manage(app::AppState {
            engine: std::sync::Mutex::new(engine::Engine::new()),
        })
        .setup(|app| {
            // This is a menu-bar-only app with no Dock icon or app menu by
            // design (see the ExitRequested handling below, which relies on
            // there being no Cmd+Q affordance to worry about). Without this,
            // macOS defaults to a regular app: a Dock icon, a menu bar, and a
            // Cmd+Q that would surface as exactly the ExitRequested we swallow.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            app::build_tray(app)?;
            app::spawn_loop(app.handle().clone());

            let h = app.handle().clone();
            app.listen_any("tt://snooze", move |_| {
                let cmds = {
                    let state = h.state::<app::AppState>();
                    let mut engine = state.engine.lock().unwrap();
                    engine.on_user(engine::UserEvent::Snooze, app::wall_ms())
                };
                app::dispatch(&h, cmds);
            });
            let h2 = app.handle().clone();
            app.listen_any("tt://skip", move |_| {
                let cmds = {
                    let state = h2.state::<app::AppState>();
                    let mut engine = state.engine.lock().unwrap();
                    engine.on_user(engine::UserEvent::Skip, app::wall_ms())
                };
                app::dispatch(&h2, cmds);
            });

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_handle, event| {
            // This app never keeps a window open (`app.windows: []`, and the
            // overlay windows are created/destroyed on demand): Tauri's runtime
            // otherwise treats "last window closed" as a request to quit the
            // whole app, which would kill this tray app the moment a break's
            // overlay window closes. That is the *only* source of a `code: None`
            // ExitRequested this app expects to see, and it is safe to swallow
            // unconditionally: `build_tray` (see app.rs) exits the process
            // during `setup`, before this closure can ever run, if it cannot
            // build a tray with a working Quit item -- so by the time we get
            // here, a real exit affordance is guaranteed to exist. On macOS,
            // `set_activation_policy(Accessory)` above additionally removes the
            // Dock icon and app menu, so there is no Cmd+Q to reach this path
            // in the first place; the swallow below is then purely the same
            // window-close backstop as on Linux/Windows. A deliberate,
            // programmatic exit (`AppHandle::exit`, used by the tray's "Quit"
            // item) always carries `code: Some(code)` and is never swallowed.
            if let tauri::RunEvent::ExitRequested { code: None, api, .. } = event {
                api.prevent_exit();
            }
        });
}
