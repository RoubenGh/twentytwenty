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
    /// Real seconds the overlay has been on screen this break, counted
    /// regardless of input. Bounds the `BREAK_INPUT_GRACE_SECS` hold below.
    break_on_screen_secs: u64,
    defer_secs: u64,
    paused_until_ms: Option<u64>,
    /// Set when a manual break was started from `Paused`, so the break ends
    /// back in `Paused` instead of silently cancelling the pause.
    resume_paused_after_break: bool,
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
            break_on_screen_secs: 0,
            defer_secs: 0,
            paused_until_ms: None,
            resume_paused_after_break: false,
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
        let (step, gapped) = self.advance_clock(now_ms, wall_ms);
        let active = !gapped
            && !s.locked
            && (s.idle_seconds < ACTIVE_GRACE_SECS || s.display_held_awake);
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
                // Counted unconditionally, unlike `break_remaining`: this is
                // the ceiling that makes the input hold above bounded. Without
                // it, a user who never stops touching the machine holds a
                // fullscreen, always-on-top, click-swallowing window open
                // forever, with no exit short of killing the process.
                self.break_on_screen_secs += step;
                if self.break_remaining == 0
                    || self.break_on_screen_secs >= BREAK_ON_SCREEN_CEILING_SECS
                {
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
                    if wall_ms >= until {
                        self.state = State::Accumulating;
                        self.paused_until_ms = None;
                    }
                }
            }
            State::BreakDue => {}
        }

        // Re-evaluate BreakDue immediately in the same tick. This is deliberate: it allows
        // a break to fire on the tick the interval completes, not one tick later.
        if self.state == State::BreakDue {
            if s.presenting {
                self.defer_secs += step;
                // Use strictly-greater-than (not >=) because the transition into BreakDue
                // runs this same block in the same tick, so defer_secs is already 1 before
                // any real deferral time has elapsed. This fires at exactly DEFER_LIMIT_SECS
                // of real deferral, not at DEFER_LIMIT_SECS - 1.
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
                self.break_on_screen_secs = 0;
                cmds.push(Command::ShowOverlay);
                cmds.push(Command::UpdateOverlay {
                    remaining_secs: BREAK_LENGTH_SECS,
                });
            }
        }

        cmds.push(Command::TrayStatus(self.tray_status()));
        cmds
    }

    /// Applies a tray/overlay-driven user action.
    ///
    /// `wall_ms` is WALL-CLOCK milliseconds, unlike `tick`'s monotonic
    /// `now_ms`, and that difference is deliberate: the only thing it is used
    /// for is the "Pause for 1 hour" deadline, which is compared against
    /// `tick`'s own `wall_ms` argument. Pause deadlines are wall-clock by
    /// design, so that pausing for an hour and then suspending the machine
    /// for that hour still un-pauses on resume. Feeding a monotonic value in
    /// here would make the deadline land in the past on the very next tick
    /// and expire the pause immediately.
    ///
    /// Every arm that can leave `OnBreak` must emit `HideOverlay`: the
    /// overlay is a fullscreen, always-on-top, click-swallowing window and
    /// nothing else takes it down. Arms that are a no-op in the current state
    /// deliberately leave the overlay alone, because they also leave the
    /// break running.
    pub fn on_user(&mut self, ev: UserEvent, wall_ms: u64) -> Vec<Command> {
        let mut cmds = Vec::new();
        match ev {
            // Only meaningful while a break is on screen or pending. From
            // `Accumulating` this used to set `Snoozed`, which makes the
            // snooze timer run and fires a break after SNOOZE_LENGTH_SECS:
            // the item that promises to postpone a break would bring one
            // forward instead.
            UserEvent::Snooze if matches!(self.state, State::OnBreak | State::BreakDue) => {
                self.state = State::Snoozed;
                self.reset_episode();
                cmds.push(Command::HideOverlay);
            }
            UserEvent::Skip => {
                self.state = State::Accumulating;
                self.bank_secs = 0;
                self.reset_episode();
                cmds.push(Command::HideOverlay);
            }
            UserEvent::BreakNow => {
                // A manual break taken while paused must not quietly cancel
                // the pause: `finish_break` reads this and goes back to
                // `Paused` (with its deadline, if any, untouched). Only
                // compute the flag when not already on a break: BreakNow is
                // reachable while OnBreak (the tray item stays enabled), and
                // recomputing against the current state there would read
                // `OnBreak`, not the original `Paused`, and silently erase
                // the captured intent.
                if self.state != State::OnBreak {
                    self.resume_paused_after_break = self.state == State::Paused;
                }
                // A repeated BreakNow while already OnBreak restarts the
                // countdown (break_remaining is unconditionally reset below)
                // rather than being a no-op: each click means "a full break
                // starts now," which is simpler to reason about than a
                // no-op that has to special-case "already on a break."
                self.state = State::OnBreak;
                self.break_remaining = BREAK_LENGTH_SECS;
                self.break_on_screen_secs = 0;
                cmds.push(Command::ShowOverlay);
                cmds.push(Command::UpdateOverlay {
                    remaining_secs: BREAK_LENGTH_SECS,
                });
            }
            UserEvent::Pause { for_ms } => {
                self.state = State::Paused;
                self.bank_secs = 0;
                self.reset_episode();
                self.paused_until_ms = for_ms.map(|d| wall_ms + d);
                cmds.push(Command::HideOverlay);
            }
            // Only meaningful while paused. From any other state this wiped
            // `bank_secs`, silently throwing away up to a full work interval
            // of accumulated screen time.
            UserEvent::Resume if self.state == State::Paused => {
                self.state = State::Accumulating;
                self.paused_until_ms = None;
                self.bank_secs = 0;
                self.reset_episode();
                cmds.push(Command::HideOverlay);
            }
            // Snooze while not on/awaiting a break, Resume while not paused.
            UserEvent::Snooze | UserEvent::Resume => {}
        }
        cmds.push(Command::TrayStatus(self.tray_status()));
        cmds
    }

    fn finish_break(&mut self, cmds: &mut Vec<Command>) {
        self.state = if self.resume_paused_after_break {
            State::Paused
        } else {
            State::Accumulating
        };
        self.bank_secs = 0;
        self.reset_episode();
        cmds.push(Command::HideOverlay);
    }

    /// Clears the per-episode counters (and the pause-return flag) that every
    /// state change shares. `bank_secs` is deliberately NOT touched here:
    /// which events keep the accumulated screen time differs per event, so
    /// each arm decides that for itself.
    fn reset_episode(&mut self) {
        self.away_secs = 0;
        self.snooze_secs = 0;
        self.break_remaining = 0;
        self.break_on_screen_secs = 0;
        self.resume_paused_after_break = false;
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

    /// Returns (seconds to advance, whether this was an abnormal gap).
    /// A gap means the machine slept or the tick loop stalled; in both cases the
    /// user was not at the screen, so the time is credited to the away timer.
    fn advance_clock(&mut self, now_ms: u64, wall_ms: u64) -> (u64, bool) {
        let mono = self.last_mono_ms.map(|p| now_ms.saturating_sub(p)).unwrap_or(TICK_MS);
        let wall = self.last_wall_ms.map(|p| wall_ms.saturating_sub(p)).unwrap_or(TICK_MS);
        self.last_mono_ms = Some(now_ms);
        self.last_wall_ms = Some(wall_ms);
        let elapsed = mono.max(wall);
        let gapped = elapsed > 2 * TICK_MS;
        ((elapsed / TICK_MS).max(1), gapped)
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

    /// A user who keeps touching the keyboard must not be able to hold the
    /// overlay on screen forever. The overlay is fullscreen, always-on-top and
    /// swallows clicks, so an unbounded hold is an input trap with no exit --
    /// and mashing keys is exactly what someone does when a break surprises
    /// them, so the pathological input is also the instinctive one. Reported
    /// from the field as "it completely locks my screen, I have to restart my
    /// laptop".
    #[test]
    fn constant_input_cannot_hold_the_overlay_open_forever() {
        let mut e = Engine::new();
        let mut t = run(&mut e, typing(), WORK_INTERVAL_SECS, 0);
        assert_eq!(e.state(), State::OnBreak);

        let mut saw_hide = false;
        // Twice the ceiling: if the break has not ended by then it never will.
        for _ in 0..(2 * BREAK_ON_SCREEN_CEILING_SECS) {
            t += TICK_MS;
            if e.tick(typing(), t, t).contains(&Command::HideOverlay) {
                saw_hide = true;
                break;
            }
        }
        assert!(
            saw_hide,
            "the overlay never came down under continuous input: it is an input trap"
        );
        assert_ne!(e.state(), State::OnBreak);
    }

    /// The ceiling is a safety valve, not the normal path: a break taken the
    /// ordinary way (stop touching the machine, look away) still ends on
    /// BREAK_LENGTH_SECS, well before the ceiling.
    #[test]
    fn the_ceiling_does_not_shorten_an_ordinary_break() {
        let mut e = Engine::new();
        let mut t = run(&mut e, typing(), WORK_INTERVAL_SECS, 0);
        for _ in 0..(BREAK_LENGTH_SECS - 1) {
            t += TICK_MS;
            assert!(!e.tick(away(), t, t).contains(&Command::HideOverlay));
        }
        t += TICK_MS;
        assert!(e.tick(away(), t, t).contains(&Command::HideOverlay));
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

    /// Every `UserEvent`, applied while a break is on screen. The overlay is
    /// a fullscreen, always-on-top, click-swallowing window that nothing but
    /// `HideOverlay` takes down, so leaving `OnBreak` without emitting it
    /// strands it over the user's desktop for up to a full work interval.
    /// Events that are a no-op in `OnBreak` are allowed to emit nothing --
    /// they also leave the break (and therefore the overlay) running.
    #[test]
    fn every_user_event_that_ends_a_break_hides_the_overlay() {
        let events = [
            UserEvent::Snooze,
            UserEvent::Skip,
            UserEvent::BreakNow,
            UserEvent::Pause { for_ms: None },
            UserEvent::Pause {
                for_ms: Some(60 * TICK_MS),
            },
            UserEvent::Resume,
        ];
        for ev in events {
            let mut e = Engine::new();
            let t = run(&mut e, typing(), WORK_INTERVAL_SECS, 0);
            assert_eq!(e.state(), State::OnBreak);
            let cmds = e.on_user(ev, t);
            if e.state() != State::OnBreak {
                assert!(
                    cmds.contains(&Command::HideOverlay),
                    "{ev:?} left OnBreak without hiding the overlay"
                );
            }
        }
    }

    #[test]
    fn resume_during_a_break_leaves_the_break_running() {
        let mut e = Engine::new();
        let t = run(&mut e, typing(), WORK_INTERVAL_SECS, 0);
        e.on_user(UserEvent::Resume, t);
        assert_eq!(e.state(), State::OnBreak, "Resume is not a break dismissal");
        // And the break still completes on its own, taking the overlay with it.
        let mut saw_hide = false;
        let mut t = t;
        for _ in 0..BREAK_LENGTH_SECS {
            t += TICK_MS;
            if e.tick(away(), t, t).contains(&Command::HideOverlay) {
                saw_hide = true;
            }
        }
        assert!(saw_hide);
        assert_eq!(e.state(), State::Accumulating);
    }

    #[test]
    fn resume_while_working_keeps_the_bank() {
        let mut e = Engine::new();
        let t = run(&mut e, typing(), 600, 0);
        e.on_user(UserEvent::Resume, t);
        assert_eq!(e.state(), State::Accumulating);
        assert_eq!(e.bank_secs(), 600, "Resume must not throw away screen time");
    }

    #[test]
    fn resume_ends_a_pause() {
        let mut e = Engine::new();
        e.on_user(UserEvent::Pause { for_ms: None }, 0);
        e.on_user(UserEvent::Resume, 0);
        assert_eq!(e.state(), State::Accumulating);
        run(&mut e, typing(), 60, 0);
        assert_eq!(e.bank_secs(), 60, "resuming must start counting again");
    }

    #[test]
    fn snooze_while_working_does_not_bring_a_break_forward() {
        let mut e = Engine::new();
        let t = run(&mut e, typing(), 60, 0);
        e.on_user(UserEvent::Snooze, t);
        assert_eq!(e.state(), State::Accumulating, "nothing to snooze");
        assert_eq!(e.bank_secs(), 60);
        // The snooze length must not become a shortcut to a break.
        run(&mut e, typing(), SNOOZE_LENGTH_SECS + 5, t);
        assert_eq!(e.state(), State::Accumulating);
    }

    #[test]
    fn a_manual_break_while_paused_goes_back_to_paused() {
        let mut e = Engine::new();
        e.on_user(UserEvent::Pause { for_ms: None }, 0);
        e.on_user(UserEvent::BreakNow, 0);
        assert_eq!(e.state(), State::OnBreak);
        let t = run(&mut e, away(), BREAK_LENGTH_SECS, 0);
        assert_eq!(e.state(), State::Paused, "a manual break must not un-pause");
        run(&mut e, typing(), 600, t);
        assert_eq!(e.bank_secs(), 0, "still paused, still not counting");
    }

    #[test]
    fn a_reentrant_break_now_during_a_paused_originated_break_still_returns_to_paused() {
        let mut e = Engine::new();
        e.on_user(UserEvent::Pause { for_ms: None }, 0);
        e.on_user(UserEvent::BreakNow, 0);
        assert_eq!(e.state(), State::OnBreak);
        // Reentrant: "Take a break now" clicked again mid-break. This must
        // not recompute resume_paused_after_break against the now-current
        // OnBreak state and overwrite the paused intent captured above.
        e.on_user(UserEvent::BreakNow, 0);
        assert_eq!(e.state(), State::OnBreak);
        let t = run(&mut e, away(), BREAK_LENGTH_SECS, 0);
        assert_eq!(e.state(), State::Paused, "a reentrant manual break must not un-pause");
        run(&mut e, typing(), 600, t);
        assert_eq!(e.bank_secs(), 0, "still paused, still not counting");
    }

    #[test]
    fn a_manual_break_while_working_returns_to_accumulating() {
        let mut e = Engine::new();
        let t = run(&mut e, typing(), 600, 0);
        e.on_user(UserEvent::BreakNow, t);
        run(&mut e, away(), BREAK_LENGTH_SECS, t);
        assert_eq!(e.state(), State::Accumulating);
        assert_eq!(e.bank_secs(), 0, "a manual break restarts the interval");
    }

    #[test]
    fn break_now_shows_the_overlay_immediately() {
        let mut e = Engine::new();
        let cmds = e.on_user(UserEvent::BreakNow, 0);
        assert!(cmds.contains(&Command::ShowOverlay));
        assert_eq!(e.state(), State::OnBreak);
    }

    #[test]
    fn sleeping_the_machine_counts_as_a_break_not_screen_time() {
        let mut e = Engine::new();
        let t = run(&mut e, typing(), 600, 0);
        assert_eq!(e.bank_secs(), 600);
        // Monotonic time stalls during suspend; wall clock jumps an hour.
        let mono = t + TICK_MS;
        let wall = t + 3_600 * TICK_MS;
        e.tick(typing(), mono, wall);
        assert_eq!(e.bank_secs(), 0, "an hour asleep is a break");
    }

    #[test]
    fn a_stalled_loop_does_not_inflate_the_bank() {
        let mut e = Engine::new();
        let t = run(&mut e, typing(), 60, 0);
        // Both clocks jump: the loop was starved, the user was not at the screen.
        let j = t + 300 * TICK_MS;
        e.tick(typing(), j, j);
        assert_eq!(e.bank_secs(), 0);
    }

    #[test]
    fn an_ordinary_tick_is_never_treated_as_a_gap() {
        let mut e = Engine::new();
        run(&mut e, typing(), 120, 0);
        assert_eq!(e.bank_secs(), 120);
    }
}
