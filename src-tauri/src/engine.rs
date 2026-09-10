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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserEvent {
    Snooze,
    Skip,
    BreakNow,
    Pause { for_ms: Option<u64> },
    Resume,
}

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

        match self.state {
            State::Accumulating => {
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
                if self.bank_secs >= WORK_INTERVAL_SECS {
                    self.state = State::BreakDue;
                    self.defer_secs = 0;
                }
            }
            State::OnBreak => {
                if s.idle_seconds >= BREAK_INPUT_GRACE_SECS {
                    self.break_remaining = self.break_remaining.saturating_sub(step);
                }
                if self.break_remaining == 0 {
                    self.finish_break(&mut cmds);
                } else {
                    cmds.push(Command::UpdateOverlay {
                        remaining_secs: self.break_remaining,
                    });
                }
            }
            State::Snoozed => {
                if active {
                    self.snooze_secs += step;
                    self.away_secs = 0;
                    if self.snooze_secs >= SNOOZE_LENGTH_SECS {
                        self.state = State::BreakDue;
                        self.snooze_secs = 0;
                        self.defer_secs = 0;
                    }
                } else {
                    self.away_secs += step;
                    if self.away_secs >= NATURAL_BREAK_SECS {
                        self.state = State::Accumulating;
                        self.bank_secs = 0;
                        self.away_secs = 0;
                        self.snooze_secs = 0;
                    }
                }
            }
            State::Paused => {
                if let Some(until) = self.paused_until_ms {
                    if now_ms >= until {
                        self.state = State::Accumulating;
                        self.paused_until_ms = None;
                    }
                }
            }
            State::BreakDue => {}
        }

        if self.state == State::BreakDue {
            if s.presenting {
                self.defer_secs += step;
                if self.defer_secs > DEFER_LIMIT_SECS {
                    cmds.push(Command::Notify {
                        title: "Time to rest your eyes".into(),
                        body: "Look at something 20 feet away for 20 seconds.".into(),
                    });
                    self.state = State::Accumulating;
                    self.bank_secs = 0;
                    self.away_secs = 0;
                    self.defer_secs = 0;
                }
            } else {
                self.state = State::OnBreak;
                self.break_remaining = BREAK_LENGTH_SECS;
                cmds.push(Command::ShowOverlay);
                cmds.push(Command::UpdateOverlay {
                    remaining_secs: BREAK_LENGTH_SECS,
                });
            }
        }

        cmds.push(Command::TrayStatus(self.tray_status()));
        cmds
    }

    pub fn on_user(&mut self, ev: UserEvent, now_ms: u64) -> Vec<Command> {
        let mut cmds = Vec::new();
        match ev {
            UserEvent::Snooze => {
                self.state = State::Snoozed;
                self.snooze_secs = 0;
                self.away_secs = 0;
                self.break_remaining = 0;
                cmds.push(Command::HideOverlay);
            }
            UserEvent::Skip => {
                self.state = State::Accumulating;
                self.bank_secs = 0;
                self.away_secs = 0;
                self.snooze_secs = 0;
                self.break_remaining = 0;
                cmds.push(Command::HideOverlay);
            }
            UserEvent::BreakNow => {
                self.state = State::OnBreak;
                self.break_remaining = BREAK_LENGTH_SECS;
                cmds.push(Command::ShowOverlay);
                cmds.push(Command::UpdateOverlay {
                    remaining_secs: BREAK_LENGTH_SECS,
                });
            }
            UserEvent::Pause { for_ms } => {
                self.state = State::Paused;
                self.bank_secs = 0;
                self.away_secs = 0;
                self.snooze_secs = 0;
                self.break_remaining = 0;
                self.paused_until_ms = for_ms.map(|d| now_ms + d);
                cmds.push(Command::HideOverlay);
            }
            UserEvent::Resume => {
                self.state = State::Accumulating;
                self.paused_until_ms = None;
                self.bank_secs = 0;
                self.away_secs = 0;
            }
        }
        cmds.push(Command::TrayStatus(self.tray_status()));
        cmds
    }

    fn finish_break(&mut self, cmds: &mut Vec<Command>) {
        self.state = State::Accumulating;
        self.bank_secs = 0;
        self.away_secs = 0;
        self.break_remaining = 0;
        cmds.push(Command::HideOverlay);
    }

    fn tray_status(&self) -> TrayStatus {
        match self.state {
            State::OnBreak => TrayStatus::Break,
            State::Snoozed => TrayStatus::Snoozed,
            State::Paused => TrayStatus::Paused,
            _ => TrayStatus::Working {
                bank_secs: self.bank_secs,
            },
        }
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

    fn presenting() -> Sample {
        Sample { idle_seconds: 0, presenting: true, ..Default::default() }
    }

    #[test]
    fn a_break_fires_after_a_full_work_interval() {
        let mut e = Engine::new();
        run(&mut e, typing(), WORK_INTERVAL_SECS, 0);
        assert_eq!(e.state(), State::OnBreak);
    }

    #[test]
    fn firing_a_break_emits_show_overlay() {
        let mut e = Engine::new();
        let mut t = 0;
        let mut cmds = Vec::new();
        for _ in 0..WORK_INTERVAL_SECS {
            t += TICK_MS;
            cmds = e.tick(typing(), t, t);
        }
        assert!(cmds.contains(&Command::ShowOverlay));
    }

    #[test]
    fn the_overlay_is_suppressed_while_presenting() {
        let mut e = Engine::new();
        let t = run(&mut e, presenting(), WORK_INTERVAL_SECS, 0);
        assert_eq!(e.state(), State::BreakDue, "must not ambush a presentation");
        run(&mut e, presenting(), 60, t);
        assert_eq!(e.state(), State::BreakDue);
    }

    #[test]
    fn deferral_gives_up_and_notifies_after_the_limit() {
        let mut e = Engine::new();
        let mut t = run(&mut e, presenting(), WORK_INTERVAL_SECS, 0);
        let mut saw_notify = false;
        for _ in 0..DEFER_LIMIT_SECS {
            t += TICK_MS;
            if e.tick(presenting(), t, t).iter().any(|c| matches!(c, Command::Notify { .. })) {
                saw_notify = true;
            }
        }
        assert!(saw_notify, "a long call must not silently disable the app");
        assert_eq!(e.state(), State::Accumulating);
        assert_eq!(e.bank_secs(), 0);
    }

    #[test]
    fn the_overlay_appears_once_presenting_ends() {
        let mut e = Engine::new();
        let t = run(&mut e, presenting(), WORK_INTERVAL_SECS, 0);
        e.tick(typing(), t + TICK_MS, t + TICK_MS);
        assert_eq!(e.state(), State::OnBreak);
    }

    #[test]
    fn the_countdown_completes_only_when_input_stops() {
        let mut e = Engine::new();
        let t = run(&mut e, typing(), WORK_INTERVAL_SECS, 0);
        // Keep typing through the whole break length: the countdown must hold.
        let t = run(&mut e, typing(), BREAK_LENGTH_SECS + 5, t);
        assert_eq!(e.state(), State::OnBreak, "typing must hold the countdown");
        // Now actually look away.
        run(&mut e, away(), BREAK_LENGTH_SECS, t);
        assert_eq!(e.state(), State::Accumulating);
        assert_eq!(e.bank_secs(), 0);
    }

    #[test]
    fn a_completed_break_hides_the_overlay() {
        let mut e = Engine::new();
        let mut t = run(&mut e, typing(), WORK_INTERVAL_SECS, 0);
        let mut saw_hide = false;
        for _ in 0..BREAK_LENGTH_SECS {
            t += TICK_MS;
            if e.tick(away(), t, t).contains(&Command::HideOverlay) {
                saw_hide = true;
            }
        }
        assert!(saw_hide);
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

    #[test]
    fn snoozing_refires_after_the_snooze_length() {
        let mut e = Engine::new();
        let t = run(&mut e, typing(), WORK_INTERVAL_SECS, 0);
        e.on_user(UserEvent::Snooze, t);
        assert_eq!(e.state(), State::Snoozed);
        let t = run(&mut e, typing(), SNOOZE_LENGTH_SECS - 5, t);
        assert_eq!(e.state(), State::Snoozed);
        run(&mut e, typing(), 5, t);
        assert_eq!(e.state(), State::OnBreak);
    }

    #[test]
    fn snooze_is_measured_in_active_time_not_wall_clock() {
        let mut e = Engine::new();
        let t = run(&mut e, typing(), WORK_INTERVAL_SECS, 0);
        e.on_user(UserEvent::Snooze, t);
        // Idle for less than a natural break: the snooze timer must not advance.
        let t = run(&mut e, away(), NATURAL_BREAK_SECS - 10, t);
        let t = run(&mut e, typing(), SNOOZE_LENGTH_SECS - 5, t);
        assert_eq!(e.state(), State::Snoozed);
        run(&mut e, typing(), 5, t);
        assert_eq!(e.state(), State::OnBreak);
    }

    #[test]
    fn a_natural_break_while_snoozed_clears_everything() {
        let mut e = Engine::new();
        let t = run(&mut e, typing(), WORK_INTERVAL_SECS, 0);
        e.on_user(UserEvent::Snooze, t);
        run(&mut e, away(), NATURAL_BREAK_SECS, t);
        assert_eq!(e.state(), State::Accumulating);
        assert_eq!(e.bank_secs(), 0);
    }

    #[test]
    fn skipping_resets_the_full_interval() {
        let mut e = Engine::new();
        let t = run(&mut e, typing(), WORK_INTERVAL_SECS, 0);
        let cmds = e.on_user(UserEvent::Skip, t);
        assert!(cmds.contains(&Command::HideOverlay));
        assert_eq!(e.state(), State::Accumulating);
        assert_eq!(e.bank_secs(), 0);
    }

    #[test]
    fn pausing_stops_accumulation_entirely() {
        let mut e = Engine::new();
        e.on_user(UserEvent::Pause { for_ms: None }, 0);
        run(&mut e, typing(), 600, 0);
        assert_eq!(e.state(), State::Paused);
        assert_eq!(e.bank_secs(), 0);
    }

    #[test]
    fn a_timed_pause_expires_on_its_own() {
        let mut e = Engine::new();
        e.on_user(UserEvent::Pause { for_ms: Some(60 * TICK_MS) }, 0);
        let t = run(&mut e, typing(), 59, 0);
        assert_eq!(e.state(), State::Paused);
        run(&mut e, typing(), 2, t);
        assert_eq!(e.state(), State::Accumulating);
    }

    #[test]
    fn break_now_shows_the_overlay_immediately() {
        let mut e = Engine::new();
        let cmds = e.on_user(UserEvent::BreakNow, 0);
        assert!(cmds.contains(&Command::ShowOverlay));
        assert_eq!(e.state(), State::OnBreak);
    }
}
