use crate::config::*;
use crate::probe::Sample;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Accumulating,
    BreakDue,
    OnBreak,
    Snoozed,
    Paused,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayStatus {
    Working { bank_secs: u64 },
    Break,
    Snoozed,
    Paused,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    ShowOverlay,
    UpdateOverlay { remaining_secs: u64 },
    HideOverlay,
    Notify { title: String, body: String },
    TrayStatus(TrayStatus),
}

#[allow(dead_code)]
pub struct Engine {
    state: State,
    bank_secs: u64,
    away_secs: u64,
    snooze_secs: u64,
    break_remaining: u64,
    defer_secs: u64,
    paused_until_ms: Option<u64>,
    last_mono_ms: Option<u64>,
    last_wall_ms: Option<u64>,
}

impl Engine {
    pub fn new() -> Self {
        Self {
            state: State::Accumulating,
            bank_secs: 0,
            away_secs: 0,
            snooze_secs: 0,
            break_remaining: 0,
            defer_secs: 0,
            paused_until_ms: None,
            last_mono_ms: None,
            last_wall_ms: None,
        }
    }

    pub fn state(&self) -> State {
        self.state
    }

    pub fn bank_secs(&self) -> u64 {
        self.bank_secs
    }

    pub fn tick(&mut self, s: Sample, now_ms: u64, wall_ms: u64) -> Vec<Command> {
        let step = self.advance_clock(now_ms, wall_ms);
        let active = !s.locked && (s.idle_seconds < ACTIVE_GRACE_SECS || s.display_held_awake);

        let mut cmds = Vec::new();
        if self.state == State::Accumulating {
            if active {
                self.bank_secs += step;
                self.away_secs = 0;
            } else {
                self.away_secs += step;
                if self.away_secs >= NATURAL_BREAK_SECS {
                    self.bank_secs = 0;
                    self.away_secs = 0;
                }
            }
        }
        cmds.push(Command::TrayStatus(TrayStatus::Working {
            bank_secs: self.bank_secs,
        }));
        cmds
    }

    /// Returns how many seconds to advance. A gap larger than two ticks means
    /// the machine slept or the loop stalled.
    fn advance_clock(&mut self, now_ms: u64, wall_ms: u64) -> u64 {
        let mono = self.last_mono_ms.map(|p| now_ms.saturating_sub(p)).unwrap_or(TICK_MS);
        let wall = self.last_wall_ms.map(|p| wall_ms.saturating_sub(p)).unwrap_or(TICK_MS);
        self.last_mono_ms = Some(now_ms);
        self.last_wall_ms = Some(wall_ms);
        (mono.max(wall) / TICK_MS).max(1)
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drives the engine for `secs` seconds with a fixed sample.
    fn run(e: &mut Engine, s: Sample, secs: u64, start_ms: u64) -> u64 {
        let mut t = start_ms;
        for _ in 0..secs {
            t += TICK_MS;
            e.tick(s, t, t);
        }
        t
    }

    fn typing() -> Sample {
        Sample { idle_seconds: 0, ..Default::default() }
    }

    fn away() -> Sample {
        Sample { idle_seconds: 300, ..Default::default() }
    }

    fn watching() -> Sample {
        Sample { idle_seconds: 300, display_held_awake: true, ..Default::default() }
    }

    #[test]
    fn typing_accumulates_screen_time() {
        let mut e = Engine::new();
        run(&mut e, typing(), 60, 0);
        assert_eq!(e.bank_secs(), 60);
    }

    #[test]
    fn watching_a_video_without_input_still_accumulates() {
        let mut e = Engine::new();
        run(&mut e, watching(), 300, 0);
        assert_eq!(e.bank_secs(), 300, "passive viewing must count as screen time");
    }

    #[test]
    fn a_short_absence_preserves_the_bank() {
        let mut e = Engine::new();
        let t = run(&mut e, typing(), 300, 0);
        run(&mut e, away(), NATURAL_BREAK_SECS - 10, t);
        assert_eq!(e.bank_secs(), 300, "a 110s absence is a pause, not a break");
    }

    #[test]
    fn a_natural_break_resets_the_bank() {
        let mut e = Engine::new();
        let t = run(&mut e, typing(), 300, 0);
        run(&mut e, away(), NATURAL_BREAK_SECS, t);
        assert_eq!(e.bank_secs(), 0, "coming back from lunch must not demand a break");
    }

    #[test]
    fn a_locked_session_never_accumulates() {
        let mut e = Engine::new();
        let s = Sample { idle_seconds: 0, display_held_awake: true, locked: true, ..Default::default() };
        run(&mut e, s, 60, 0);
        assert_eq!(e.bank_secs(), 0);
    }

    #[test]
    fn idle_below_the_grace_period_still_counts() {
        let mut e = Engine::new();
        let s = Sample { idle_seconds: ACTIVE_GRACE_SECS - 1, ..Default::default() };
        run(&mut e, s, 30, 0);
        assert_eq!(e.bank_secs(), 30);
    }
}
