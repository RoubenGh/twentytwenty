//! Every duration in the app. Nothing else may hardcode a time value.

pub const TICK_MS: u64 = 1_000;
pub const WORK_INTERVAL_SECS: u64 = 20 * 60;
pub const BREAK_LENGTH_SECS: u64 = 20;
pub const ACTIVE_GRACE_SECS: u64 = 60;
pub const NATURAL_BREAK_SECS: u64 = 2 * 60;
pub const SNOOZE_LENGTH_SECS: u64 = 5 * 60;
pub const BREAK_INPUT_GRACE_SECS: u64 = 2;
pub const DEFER_LIMIT_SECS: u64 = 10 * 60;
/// A probe reporting more idle time than this in one sample is not plausible
/// for a real desktop session; treat it as a broken probe and downgrade.
pub const MAX_PLAUSIBLE_IDLE_SECS: u64 = 24 * 60 * 60;
/// Duration of the tray's "Pause for 1 hour" menu item.
pub const PAUSE_ONE_HOUR_MS: u64 = 60 * 60 * 1_000;
