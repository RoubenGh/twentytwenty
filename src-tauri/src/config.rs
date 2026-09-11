//! Every duration in the app. Nothing else may hardcode a time value.

pub const TICK_MS: u64 = 1_000;
pub const WORK_INTERVAL_SECS: u64 = 20 * 60;
pub const BREAK_LENGTH_SECS: u64 = 20;
pub const ACTIVE_GRACE_SECS: u64 = 60;
pub const NATURAL_BREAK_SECS: u64 = 2 * 60;
pub const SNOOZE_LENGTH_SECS: u64 = 5 * 60;
pub const BREAK_INPUT_GRACE_SECS: u64 = 2;
/// Hard ceiling on how long the break overlay may stay on screen, counted in
/// real seconds from the moment it appears and regardless of what the user is
/// doing. `BREAK_INPUT_GRACE_SECS` deliberately holds the countdown while
/// input is still arriving (typing through a break is not resting your eyes),
/// but that hold must be BOUNDED: the overlay is fullscreen, always-on-top and
/// swallows clicks, so an unbounded hold is an input trap, and mashing the
/// keyboard is precisely what a surprised user does. Past this ceiling the
/// break ends on its own. Generous enough that an ordinary break (finish your
/// sentence, then look away) never reaches it.
pub const BREAK_ON_SCREEN_CEILING_SECS: u64 = 90;
/// How long the overlay page gets to prove it can paint before the overlay is
/// abandoned. The break window is fullscreen, always-on-top and swallows
/// clicks, and every control that dismisses it (Esc, Snooze, Skip) lives in
/// that page's script, so a window whose content never renders is a total
/// input trap with no exit. It is therefore created HIDDEN and only promoted
/// to a real overlay once the page emits `tt://overlay-ready`; if that never
/// arrives within this budget the windows are destroyed and the break falls
/// back to a plain notification. A missed break is an inconvenience. A locked
/// machine is not.
///
/// This has fired for real: the AppImage runtime bundles its own EGL/GTK
/// libraries, and on a host whose driver disagrees WebKit dies with
/// "Could not create default EGL display: EGL_BAD_PARAMETER. Aborting..."
/// before a single frame or a single line of JavaScript. See
/// `docs/findings/2026-09-11-overlay-input-trap.md`.
pub const OVERLAY_READY_TIMEOUT_MS: u64 = 2_500;
pub const DEFER_LIMIT_SECS: u64 = 10 * 60;
/// Ceiling applied to a probe's reported idle time. More than this in one
/// sample is not plausible for a real desktop session, but it is also not
/// evidence of a broken probe: a machine with sleep disabled genuinely
/// reports a whole weekend of idle time, and a wrapped 32-bit counter can
/// report anything at all. Such samples are CLAMPED to this value, never
/// used to downgrade the probe: everything above `ACTIVE_GRACE_SECS` is
/// already "not at the screen" as far as the engine is concerned, so
/// clamping loses no information the engine could have acted on. Only a
/// probe `Err` downgrades the session to the fallback probe.
pub const MAX_PLAUSIBLE_IDLE_SECS: u64 = 24 * 60 * 60;
/// Duration of the tray's "Pause for 1 hour" menu item.
pub const PAUSE_ONE_HOUR_MS: u64 = 60 * 60 * 1_000;
