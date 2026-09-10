use super::{ActivityProbe, Sample};

/// Used when a platform probe fails. Reports a permanently non-idle session,
/// which degrades the app to a plain interval timer rather than letting a
/// broken sensor stop breaks from ever firing.
pub struct FallbackProbe;

impl FallbackProbe {
    pub fn new() -> Self {
        Self
    }
}

impl Default for FallbackProbe {
    fn default() -> Self {
        Self::new()
    }
}

impl ActivityProbe for FallbackProbe {
    fn sample(&mut self) -> anyhow::Result<Sample> {
        Ok(Sample {
            idle_seconds: 0,
            display_held_awake: false,
            presenting: false,
            locked: false,
        })
    }

    fn name(&self) -> &'static str {
        "fallback"
    }
}
