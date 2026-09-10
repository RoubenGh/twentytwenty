pub mod fallback;
#[cfg(target_os = "linux")]
pub mod linux;

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
}
