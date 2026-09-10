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
            app::build_tray(app)?;
            app::spawn_loop(app.handle().clone());

            let h = app.handle().clone();
            app.listen_any("tt://snooze", move |_| {
                let cmds = {
                    let state = h.state::<app::AppState>();
                    let mut engine = state.engine.lock().unwrap();
                    engine.on_user(engine::UserEvent::Snooze, 0)
                };
                app::dispatch(&h, cmds);
            });
            let h2 = app.handle().clone();
            app.listen_any("tt://skip", move |_| {
                let cmds = {
                    let state = h2.state::<app::AppState>();
                    let mut engine = state.engine.lock().unwrap();
                    engine.on_user(engine::UserEvent::Skip, 0)
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
            // overlay window closes. Only let a deliberate, programmatic exit
            // (`AppHandle::exit`, used by the tray's "Quit" item, which passes
            // `Some(code)`) through; swallow the window-driven one (`None`).
            if let tauri::RunEvent::ExitRequested { code: None, api, .. } = event {
                api.prevent_exit();
            }
        });
}
