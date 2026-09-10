#![cfg(target_os = "linux")]

use twentytwenty_lib::probe::{linux::LinuxProbe, ActivityProbe};

/// Requires a real desktop session (D-Bus session bus + a running Wayland
/// compositor). Skips itself when run headlessly, so CI stays green.
fn session_available() -> bool {
    std::env::var("DBUS_SESSION_BUS_ADDRESS").is_ok() && std::env::var("WAYLAND_DISPLAY").is_ok()
}

#[test]
fn reports_a_plausible_idle_time() {
    if !session_available() {
        eprintln!("skipping: no session bus / Wayland display");
        return;
    }
    let mut p = LinuxProbe::new().expect("probe constructs in a real session");
    let s = p.sample().expect("probe samples without error");
    assert!(
        s.idle_seconds < 86_400,
        "idle time must be plausible, got {}",
        s.idle_seconds
    );
    assert_eq!(p.name(), "linux");
}

#[test]
fn idle_time_grows_while_untouched() {
    if !session_available() {
        eprintln!("skipping: no session bus / Wayland display");
        return;
    }
    let mut p = LinuxProbe::new().unwrap();
    let first = p.sample().unwrap().idle_seconds;
    std::thread::sleep(std::time::Duration::from_secs(3));
    let second = p.sample().unwrap().idle_seconds;
    assert!(
        second >= first,
        "idle must not move backwards: {first} then {second}"
    );
}
