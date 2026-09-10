pub mod fallback;
#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "windows")]
pub mod windows;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Sample {
    /// Seconds since the last keyboard or mouse input.
    pub idle_seconds: u64,
    /// Something is asking the OS to keep the display awake (a video is playing).
    pub display_held_awake: bool,
    /// A fullscreen app, presentation mode, or an active screen capture.
    pub presenting: bool,
    /// The session is locked.
    pub locked: bool,
}

pub trait ActivityProbe: Send {
    fn sample(&mut self) -> anyhow::Result<Sample>;
    fn name(&self) -> &'static str;
}

/// Returns the best probe this platform can offer, falling back when the
/// platform probe cannot be constructed. Never fails.
pub fn select() -> Box<dyn ActivityProbe> {
    #[cfg(target_os = "linux")]
    {
        match linux::LinuxProbe::new() {
            Ok(p) => return Box::new(p),
            Err(e) => log::warn!("linux probe unavailable, falling back: {e}"),
        }
    }
    #[cfg(target_os = "windows")]
    {
        match windows::WindowsProbe::new() {
            Ok(p) => return Box::new(p),
            Err(e) => log::warn!("windows probe unavailable, falling back: {e}"),
        }
    }
    #[cfg(target_os = "macos")]
    {
        match macos::MacosProbe::new() {
            Ok(p) => return Box::new(p),
            Err(e) => log::warn!("macos probe unavailable, falling back: {e}"),
        }
    }
    Box::new(fallback::FallbackProbe::new())
}

/// Parse `pmset -g assertions` output to extract display-held and presenting states.
///
/// Returns `(display_held_awake, presenting)`.
///
/// Parses the system-wide assertion summary to detect if `PreventUserIdleDisplaySleep`
/// is held. The summary format lists assertion types with counts at the end of each
/// line; a nonzero count means something is actively preventing idle display sleep.
/// Also detects screen sharing and screen capture keywords that indicate presenting.
///
/// These outputs are reconstructed from `pmset` documentation and observation,
/// as this code cannot be tested on a live Mac. This function is available on all
/// platforms so it can be tested thoroughly; it is used only by the macOS probe.
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub fn parse_pmset_assertions(output: &str) -> (bool, bool) {
    let output_lower = output.to_lowercase();
    let mut held = false;
    let mut presenting = false;

    for line in output_lower.lines() {
        let trimmed = line.trim();

        // Skip empty lines and section headers.
        if trimmed.is_empty()
            || trimmed.starts_with("system-wide")
            || trimmed.starts_with("listed by")
        {
            continue;
        }

        // In the system-wide summary, check if PreventUserIdleDisplaySleep has a
        // nonzero count. The format is:
        //   PreventUserIdleDisplaySleep    0
        // The count is the last whitespace-separated token.
        if trimmed.contains("preventuseridledisplaysleep") {
            if let Some(last_token) = trimmed.split_whitespace().last() {
                if last_token != "0" {
                    held = true;
                }
            }
        }

        // Detect presenting: screen sharing, screen capture, or similar.
        if trimmed.contains("screen sharing")
            || trimmed.contains("screencapture")
            || trimmed.contains("screen capture")
        {
            presenting = true;
        }
    }

    (held, presenting)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_defaults_to_fully_idle_and_unlocked() {
        let s = Sample::default();
        assert_eq!(s.idle_seconds, 0);
        assert!(!s.display_held_awake);
        assert!(!s.presenting);
        assert!(!s.locked);
    }

    #[test]
    fn fallback_probe_reports_its_name() {
        let mut p = crate::probe::fallback::FallbackProbe::new();
        assert_eq!(p.name(), "fallback");
        let s = p.sample().expect("fallback probe never fails");
        assert!(!s.display_held_awake, "fallback cannot detect assertions");
    }

    #[test]
    fn select_always_returns_a_working_probe() {
        let mut p = select();
        let s = p.sample();
        assert!(s.is_ok(), "select must never hand back a probe that errors");
    }

    #[test]
    fn parse_pmset_empty_output_is_idle() {
        let (held, presenting) = parse_pmset_assertions("");
        assert!(!held, "empty output should not hold display awake");
        assert!(!presenting, "empty output should not be presenting");
    }

    #[test]
    fn parse_pmset_garbage_input_is_idle() {
        let (held, presenting) = parse_pmset_assertions("xyzzy\nfoo bar\n");
        assert!(!held, "garbage input should not hold display awake");
        assert!(!presenting, "garbage input should not be presenting");
    }

    #[test]
    fn parse_pmset_system_summary_with_all_zero_counts_is_idle() {
        // Reconstructed from pmset documentation: system-wide summary with
        // all assertion counts at zero (nothing is holding assertions).
        let output = r#"System-wide Assertions:
   PreventUserIdleDisplaySleep    0
   PreventSystemSleep            0
   PreventUserIdleLegacyDisplaySleep 0
   PreventUserIdleDeprecatedDisplaySleep 0

Kernel assertions: 0x0

Listed by owning process:
"#;
        let (held, presenting) = parse_pmset_assertions(output);
        assert!(
            !held,
            "zero PreventUserIdleDisplaySleep count should not hold display awake"
        );
        assert!(!presenting, "all-zero output should not be presenting");
    }

    #[test]
    fn parse_pmset_nonzero_prevent_user_idle_display_sleep_holds() {
        // Reconstructed: system-wide summary with a non-zero
        // PreventUserIdleDisplaySleep count.
        let output = r#"System-wide Assertions:
   PreventUserIdleDisplaySleep    1
   PreventSystemSleep            0
   PreventUserIdleLegacyDisplaySleep 0

Kernel assertions: 0x0

Listed by owning process:
"#;
        let (held, presenting) = parse_pmset_assertions(output);
        assert!(
            held,
            "nonzero PreventUserIdleDisplaySleep count should hold display awake"
        );
        assert!(!presenting, "video playback alone is not presenting");
    }

    #[test]
    fn parse_pmset_screen_sharing_is_presenting() {
        // Reconstructed: a process name containing "screen sharing".
        let output = r#"System-wide Assertions:
   PreventUserIdleDisplaySleep    1
   PreventSystemSleep            0

Kernel assertions: 0x0

Listed by owning process:
   pid 2345(ScreenSharingAgent) PreventUserIdleDisplaySleep
"#;
        let (held, presenting) = parse_pmset_assertions(output);
        assert!(held, "screen sharing should hold display awake");
        assert!(
            presenting,
            "screen sharing should be detected as presenting"
        );
    }

    #[test]
    fn parse_pmset_screencapture_is_presenting() {
        // Reconstructed: a screen capture process.
        let output = r#"System-wide Assertions:
   PreventUserIdleDisplaySleep    1

Kernel assertions: 0x0

Listed by owning process:
   pid 3456(screencapture) PreventUserIdleDisplaySleep
"#;
        let (held, presenting) = parse_pmset_assertions(output);
        assert!(held);
        assert!(presenting, "screencapture should be detected as presenting");
    }

    #[test]
    fn parse_pmset_screen_capture_keyword_is_presenting() {
        // Reconstructed: another variant of screen capture phrasing.
        let output = r#"System-wide Assertions:
   PreventUserIdleDisplaySleep    1

Listed by owning process:
   pid 4567(MyApp) assertion name: screen capture
"#;
        let (held, presenting) = parse_pmset_assertions(output);
        assert!(held);
        assert!(presenting, "screen capture keyword should be detected");
    }
}
