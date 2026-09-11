pub mod app;
pub mod config;
pub mod engine;
pub mod overlay;
pub mod probe;

// A release binary built without `tauri/custom-protocol` is a booby trap, not
// a working app, so refuse to produce one.
//
// `cfg(dev)` is set by tauri-build whenever the `tauri` crate is compiled
// without its `custom-protocol` feature. In that mode Tauri does not embed the
// frontend at all and resolves `WebviewUrl::App` against `build.devUrl`
// instead (tauri 2.11.5, `manager/mod.rs::get_app_url`). That is correct under
// `tauri dev`, where Vite is serving on 1420. In a shipped binary nothing is
// listening there, so `overlay::show` opens a fullscreen, always-on-top,
// decorationless, click-swallowing window pointed at a dead URL: WebKit paints
// its "connection refused" page, no script runs, and the Esc handler and both
// buttons that would dismiss the overlay never exist. The user cannot click,
// cannot type past it, and cannot close it.
//
// `pnpm tauri build` passes the feature (`cargo build --bins --features
// tauri/custom-protocol --release`). A bare `cargo build --release` does not,
// and produced exactly that trap once already. This turns that mistake into a
// compile error instead of a shipped app.
#[cfg(all(not(debug_assertions), dev))]
compile_error!(
    "release build without `tauri/custom-protocol`: the break overlay would load \
     build.devUrl (nothing listens there in a shipped binary), leaving a fullscreen \
     always-on-top window with no script, no Esc handler and no buttons. \
     Build with `pnpm tauri build`, not `cargo build --release`."
);

use tauri::{Listener, Manager};
use tauri_plugin_autostart::MacosLauncher;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::init();
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|_app, _argv, _cwd| {}))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_dialog::init())
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

            // Ask, once, whether the user wants autostart -- never enable it
            // silently, and never ask again regardless of the answer. See
            // `app::maybe_ask_autostart` for the persisted "already asked"
            // marker and failure handling.
            app::maybe_ask_autostart(app);

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
