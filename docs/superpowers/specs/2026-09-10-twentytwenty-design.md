# TwentyTwenty: Design Spec

**Date:** 2026-09-10
**Status:** Approved for planning
**App id:** `dev.rouben.twentytwenty`
**Repo:** `RoubenGh/twentytwenty` (public)

## 1. Purpose

A small cross-platform desktop app that enforces the 20-20-20 rule for digital eye
strain: every 20 minutes of screen time, look at something 20 feet away for 20
seconds.

The distinguishing requirement is that it must measure **actual screen time**, not
wall-clock time. A plain repeating 20-minute alarm is explicitly rejected. If the
user steps away for lunch, the app must not demand a break the moment they return.
If the user watches a video for 15 minutes without touching the keyboard, that must
still count as screen time.

Primary user: the author, on Linux. Secondary audience: visitors to the author's
portfolio, where the app is presented as a case study.

## 2. Success criteria

1. On the author's KDE Wayland machine, the app runs all day from a tray icon and
   fires a break only after 20 minutes of genuine screen use.
2. A natural absence of two minutes or more resets the counter without a prompt.
3. Watching a video with no keyboard or mouse input continues to accumulate screen
   time.
4. The break overlay never appears while presenting, screen-sharing, or in a
   fullscreen app.
5. The break countdown only completes when the user actually stops using the
   machine.
6. Windows and macOS installers are produced by CI from the same codebase and
   install successfully, acknowledging that their runtime behavior is unverified.
7. Total monetary cost of the project is zero.

## 3. Non-goals

Explicitly out of scope for v1, each deferrable without architectural change:

- **No settings UI.** The three intervals are constants in one module.
- **No statistics, history, or database.** Nothing is recorded between runs.
- **No backend, account, sync, or telemetry.** The app is fully local and offline
  except for its update check.
- **No work-hours scheduling.**
- **No webcam or gaze detection.** Rejected as privacy-invasive and disproportionate.
- **No paid OS code signing.** Installers ship unsigned; the portfolio page
  documents the one-time Windows SmartScreen and macOS Gatekeeper bypass.

## 4. Technology

**Tauri v2**, Rust core with a minimal web UI.

Chosen because it provides first-party plugins for precisely the four platform
concerns this app has (tray icon, autostart on login, native notifications, and a
signed self-updater), produces roughly 8 MB installers for all three operating
systems from one codebase, and lets the overlay be written in HTML and CSS where a
fading dim wash with a countdown ring is trivial rather than hand-drawn.

The overlay UI is plain HTML, CSS and TypeScript built by Vite. No UI framework;
the entire interface is one screen with a countdown and two buttons.

Rejected alternatives: Electron (roughly 150 MB installed and 200 MB resident for
an app that idles almost all the time), pure Rust with egui (leaner, but the
overlay and updater both become hand-built), Python with Qt (distribution to three
operating systems is the fussiest part of the project and this makes it worse).

## 5. Architecture

```
src-tauri/src/
  probe/
    mod.rs        trait ActivityProbe -> Sample, plus selection logic
    linux.rs      ext-idle-notify-v1 / ScreenSaver DBus / logind
    windows.rs    GetLastInputInfo + SHQueryUserNotificationState
    macos.rs      CGEventSource + IOPMAssertion
    fallback.rs   input-idle only
  engine.rs       accumulator state machine, pure, no I/O
  config.rs       the interval constants
  app.rs          tray, overlay windows, autostart, updater wiring
  main.rs
ui/
  overlay.html    the break screen
  overlay.ts
```

### 5.1 The Sample boundary

Every platform reduces to the same four facts, and the engine sees nothing else:

```rust
struct Sample {
    idle_seconds: u64,       // since last keyboard or mouse input
    display_held_awake: bool,// something is preventing display sleep
    presenting: bool,        // fullscreen app, presentation mode, or screen capture
    locked: bool,            // session is locked
}
```

This boundary carries most of the design's weight. The engine is pure and
deterministic, so its behavior is verified by feeding synthetic sample sequences
rather than by waiting twenty minutes. It also contains the untestable code: only
Linux behavior can be verified by the author, so isolating Windows and macOS to one
small sensing file each means an unverified platform can report a wrong `Sample`
but can never produce wrong logic.

Every probe is fallible. Any error, or an implausible value such as an idle time
that jumps backwards, causes a permanent downgrade to `fallback.rs` for the rest of
the session, with a logged warning. Degrading to plain input-idle is always
preferable to acting on garbage.

## 6. The engine

A tick every second, driven by a monotonic clock.

### 6.1 Activity rule

```
active = NOT locked AND ((idle_seconds < ACTIVE_GRACE) OR display_held_awake)
```

The first clause is ordinary interactive use. The second is the passive-viewing
catch: every video player, browsers included, asks the operating system to prevent
display sleep while playing. Reading that assertion is permission-free on all three
platforms and is a far better signal than inspecting foreground windows, which
Wayland forbids and macOS gates behind an accessibility prompt.

A locked session is never active regardless of any assertion.

### 6.2 States

- **Accumulating.** The default. While active, the bank grows one second per tick.
  While inactive, the bank is held and an away-timer runs. If the away-timer
  reaches `NATURAL_BREAK`, the user has effectively taken a real break: the bank
  resets to zero and the away-timer clears. Shorter absences preserve the bank.
  When the bank reaches `WORK_INTERVAL`, transition to BreakDue.

- **BreakDue.** Wants to show the overlay. If `presenting` is true, it waits and
  re-checks every `DEFER_RECHECK`. After `DEFER_LIMIT` of continuous deferral it
  gives up on the overlay, emits a native notification instead, and returns to
  Accumulating with the bank reset. This prevents a long call from silently
  disabling the app for the rest of the day. Otherwise, transition to OnBreak.

- **OnBreak.** A `BREAK_LENGTH` countdown that only decrements while
  `idle_seconds >= BREAK_INPUT_GRACE`. Continued typing holds the countdown rather
  than restarting it, which is a deliberate choice: restarting punishes, holding
  simply waits. Completing the countdown resets the bank and returns to
  Accumulating. Escape or the Snooze button transitions to Snoozed. The Skip button
  resets the bank fully and returns to Accumulating.

- **Snoozed.** The bank is frozen at `WORK_INTERVAL` and a separate snooze timer
  counts active seconds using the same activity rule. When it reaches
  `SNOOZE_LENGTH`, the state returns to BreakDue. Snoozing is measured in active
  time, not wall-clock, for the same reason the main interval is. A natural absence
  of `NATURAL_BREAK` while snoozed counts as a break and returns to Accumulating
  with the bank cleared.

- **Paused.** Entered from the tray menu, either for one hour or until manually
  resumed. All accumulation stops. The tray icon reflects the paused state so it
  cannot be forgotten about.

### 6.3 Suspend and resume

Each tick compares elapsed monotonic time against elapsed wall-clock time. A
divergence greater than two ticks means the machine slept. The gap is credited to
the away-timer, not the bank, so closing a laptop for an hour is correctly treated
as a break.

### 6.4 Constants

All in `config.rs`, no UI exposure in v1:

| Constant | Value |
|---|---|
| `WORK_INTERVAL` | 20 minutes |
| `BREAK_LENGTH` | 20 seconds |
| `ACTIVE_GRACE` | 60 seconds |
| `NATURAL_BREAK` | 2 minutes |
| `SNOOZE_LENGTH` | 5 minutes of active time |
| `BREAK_INPUT_GRACE` | 2 seconds |
| `DEFER_RECHECK` | 30 seconds |
| `DEFER_LIMIT` | 10 minutes |
| `TICK` | 1 second |

## 7. Platform sensing

| Fact | Linux | Windows | macOS |
|---|---|---|---|
| Idle seconds | `ext-idle-notify-v1`, then `org.freedesktop.ScreenSaver`, then logind `IdleHint` | `GetLastInputInfo` | `CGEventSourceSecondsSinceLastEventType` |
| Display held awake | DBus inhibitor list from PowerDevil's PolicyAgent | `SHQueryUserNotificationState` reporting fullscreen or busy | `IOPMCopyAssertionsByProcess` for `PreventUserIdleDisplaySleep` |
| Presenting | Inhibitor reason heuristic (best-effort) | `QUNS_PRESENTATION_MODE`, `QUNS_RUNNING_D3D_FULL_SCREEN` | `CGDisplayIsCaptured` |
| Locked | `org.freedesktop.ScreenSaver.GetActive` | `WTS_SESSION_LOCK` notifications | `CGSSessionScreenIsLocked` |

None require elevated privileges or a macOS accessibility prompt.

`ext-idle-notify-v1` is event-based rather than a queryable counter. The Linux
probe registers notifications at `BREAK_INPUT_GRACE` and `ACTIVE_GRACE` and derives
a bucketed idle time from the resulting idled and resumed events, which is all the
engine's thresholds require.

Presentation detection on Wayland is a heuristic, because Wayland deliberately
denies clients any view of other windows. The manual pause is the reliable escape
on that platform, and this limitation is documented rather than hidden.

## 8. Interface

**While working**, the app is a tray icon and a small Rust process. No webview
exists, so idle memory cost is a few megabytes. The tray menu offers: Take a break
now, Snooze, Pause for 1 hour, Pause until I resume, Quit.

**On break**, a borderless always-on-top window is created per monitor, fades to a
dark wash at roughly 85% opacity over 400 ms, and shows a countdown ring with the
instruction to look at something 20 feet away, plus Snooze (also bound to Escape)
and Skip. When the break ends the windows are destroyed and the webview is torn
down with them.

**Autostart** on login via Tauri's autostart plugin, enabled by default on first
run.

## 9. Principal risk

Wayland does not permit an application to position or force its own windows. KWin
generally honors fullscreen and always-on-top requests, but multi-monitor overlay
placement is exactly the class of thing that works under X11 and fails under
Wayland.

**Mitigation:** the first task in the implementation plan is a throwaway probe that
puts a fullscreen always-on-top window on the author's KDE Wayland session and
confirms its behavior, before anything is built on that assumption. If it cannot be
made to work, the documented fallback is a large centered always-on-top window
rather than a full-screen wash. This risk is retired before the engine is written.

## 10. Distribution

A public GitHub repository with an Actions matrix building on Ubuntu, Windows and
macOS runners, producing `.deb` and `.AppImage`, `.msi` and `.exe`, and `.dmg`,
attached to a GitHub Release. This is free and is the only way to obtain genuine
Windows and macOS builds without owning those machines.

Tauri's updater points directly at the release assets and verifies them with a
self-generated minisign key. That key is free and unrelated to paid OS code-signing
certificates; it proves an update was not tampered with, but does not suppress the
first-install warnings on Windows and macOS.

The update channel deliberately does not depend on the portfolio site, so a
portfolio redeploy can never break an installed app's updater.

## 11. Testing

- **Engine unit tests** carry the bulk of the verification. Synthetic `Sample`
  sequences make twenty-minute intervals, natural-break resets, snooze arithmetic,
  deferral during presentations, and suspend-resume clock jumps into fast
  deterministic tests.
- **Linux probe integration tests** run against the real session, asserting that
  idle time grows when untouched and that a playing video registers as an
  inhibitor.
- **CI** compiles and runs engine tests on all three platforms. Probe integration
  tests run on Linux only.
- **The overlay** gets a written manual smoke checklist, because no test can
  honestly verify a compositor's behavior.

## 12. Follow-on work

The portfolio case study at `rouben.dev/work/twentytwenty` is a **separate
sub-project** with its own design cycle. It is a contained change to the existing
`RoubenGh/Portfolio` repository, following the established
`src/app/work/<slug>/{page.tsx,content.tsx}` pattern alongside the `ai-ticketing`
and `rla-studios` case studies, plus an entry in `Work.tsx` and screenshots in
`public/images`.

It is deliberately sequenced after the app, because it needs real screenshots of a
working overlay. A case study designed around imagined screenshots gets rewritten
the moment the real ones exist.
