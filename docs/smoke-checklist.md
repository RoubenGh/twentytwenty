# Manual smoke checklist

Run this against a release build before tagging. No test suite can honestly
verify a compositor's or OS's real behavior, so this is what stands in for
one. Every line describes something you can directly observe (see it happen,
watch a value change): nothing here is inferred or assumed.

To make the intervals practical to wait out, temporarily lower
`WORK_INTERVAL_SECS` (and optionally `BREAK_LENGTH_SECS`) in
`src-tauri/src/config.rs` before building, and restore them before tagging.
Do not run this against your everyday session without doing that first: the
real values mean a 20-minute wait per check.

1. **Launch.** A tray icon appears. No window opens.
2. **Tooltip.** Hovering the tray icon shows a tooltip counting down the
   minutes remaining until the next break.
3. **Overlay appears everywhere, on top.** With the work interval lowered,
   the break overlay fades in on every connected monitor and stays above
   other windows (click into another app; the overlay does not go away).
4. **Typing holds the break.** During the break, type continuously. The
   countdown holds (does not decrease) and the "Stop typing to start the
   countdown" hint is visible.
5. **Hands off completes the break.** Leave the keyboard and mouse alone
   during a break. The countdown reaches zero on its own and the overlay
   closes.
6. **Escape snoozes.** Press Escape during a break. The overlay closes and
   does not reappear until the snooze interval of active use has passed.
7. **Skip resets fully.** Click Skip during a break. The overlay closes and
   the tray tooltip shows the full work interval remaining, not a partial
   one.
8. **Fullscreen video keeps the clock running.** Play a fullscreen video
   with sound and take your hands off the keyboard and mouse for at least
   90 seconds. Confirm the tray tooltip's countdown keeps decreasing the
   whole time, instead of freezing or resetting as it would for a plain
   idle timer. This is the behavior that distinguishes this app from a
   20-minute alarm, so check it on every release, not just once.
9. **Lock and return.** Lock the session and leave it locked for at least 3
   minutes, then unlock. The tray tooltip shows the full work interval
   remaining on return. (Lock detection itself has already been verified in
   both directions on Linux, so this check is a regression guard, not an
   open question.)
10. **Suspend and resume.** Suspend the machine and let it sleep for at
    least a minute, then resume. The tray tooltip shows the full work
    interval remaining on return, the same as a natural break.
11. **Pause for 1 hour.** Choose "Pause for 1 hour" from the tray menu. The
    tooltip reads paused, and it does not change or count down while you
    continue using the machine.
12. **Quit actually quits.** Choose "Quit" from the tray menu. The tray icon
    disappears, the process is gone (check with `ps`/Task Manager/Activity
    Monitor, no `twentytwenty` process remains), and no window is left
    behind. This has already regressed once in development, in a way that
    made the app exit after its first break ever fired instead of staying
    running. Re-check it every time, not just after touching tray code.
13. **Docked multi-monitor check: run this one while docked to external
    monitors.** This is the check the project has never been able to run:
    every development and testing machine involved had only a single
    built-in display. With at least one external monitor connected, trigger
    a break and confirm the overlay covers every connected screen, not just
    the laptop's built-in display or whichever one is primary. If any
    monitor is left uncovered, or the overlay is mispositioned on one, that
    is a real, previously-unverified defect, not a known limitation.
