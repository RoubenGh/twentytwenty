# TwentyTwenty

A small cross-platform desktop app that enforces the 20-20-20 rule for digital
eye strain: every 20 minutes of screen time, look at something 20 feet away
for 20 seconds. It lives in the tray, does its counting, and gets out of the
way otherwise.

Downloads: [github.com/RoubenGh/twentytwenty/releases/tag/v0.1.0](https://github.com/RoubenGh/twentytwenty/releases/tag/v0.1.0)

## Why this is not a plain 20-minute timer

A repeating alarm every 20 minutes was the first idea, and it was rejected on
purpose. It nags you the moment you get back from lunch, and it has no idea
whether you're actually looking at the screen.

TwentyTwenty instead tries to measure actual screen time. Every second, it
asks the operating system four things: how long since your last keystroke or
mouse movement, whether something is currently telling the OS not to let the
display sleep, whether you're presenting or screen-sharing, and whether the
session is locked. From those four facts it decides whether you're "active"
using one rule:

```
active = session is not locked
          AND ( you gave input in the last 60 seconds
                OR something is holding the display awake )
```

The second half of that OR is the point of the whole project. Every video
player, browser included, asks the OS to prevent display sleep while it's
playing, using exactly the same mechanism a presentation app or screen-share
uses. Reading that signal is free, doesn't require any special permission,
and is a far better proxy for "the user is looking at the screen" than
watching for keyboard and mouse activity. So watching a 15-minute video
without touching anything counts as 15 minutes of screen time, the same as
if you'd been typing the whole time. Stepping away from your desk, with
nothing playing, genuinely stops the clock.

Locking your session always overrides everything else: a locked machine
never accumulates screen time, no matter what's still running behind the
lock screen.

### The natural-break reset

If you stop being active for two minutes or more, the accumulated screen
time bank resets to zero rather than just pausing. A short pause (getting up
for coffee, glancing away) doesn't cost you anything, but a real break
(lunch, a meeting, closing the laptop) starts the 20 minutes over. This is
why the app can run all day without ever nagging you the instant you sit
back down after being away for a while: the two-minute threshold is the line
between "still basically at your desk" and "actually took a break."

The same idea applies to suspend and resume: if the machine's monotonic
clock and wall clock diverge (the classic signature of a sleep/resume
cycle), the gap is credited to the away-timer, so closing your laptop for an
hour is correctly treated as a break, not as an hour of screen time.

## Break behavior

Once the 20-minute bank fills, a break overlay fades in on every connected
monitor: a dark, fullscreen, always-on-top wash with a 20-second countdown
ring. The countdown only ticks down while you're not typing: if you keep
typing through the break, the countdown holds rather than restarting, and
resumes counting the moment you stop. Escape or the Snooze button dismisses
the overlay for 5 minutes of further active use before it reappears; Skip
dismisses it and starts the full 20-minute interval over.

If a break becomes due while you're presenting or screen-sharing, the
overlay is held back and rechecked periodically rather than ambushing you
mid-presentation. If that goes on for more than 10 minutes, the app gives up
waiting, sends a native notification instead, and resets the counter, so a
long call can't silently disable the app for the rest of the day.

The tray menu also offers "Take a break now," "Pause for 1 hour," and
"Pause until I resume," for when the automatic behavior isn't what you want
right now.

None of the three intervals (work interval, break length, snooze length) are
user-configurable in this version: they're constants in
`src-tauri/src/config.rs`.

## Installing

Builds are **unsigned**. Code signing on Windows and macOS costs real money
(a few hundred dollars a year), and this is a personal, free project: the
binaries aren't sketchy, the certificates just weren't purchased. Each
platform's OS will warn you about that on first launch; here's how to get
past it.

### Linux

Download from the [release page](https://github.com/RoubenGh/twentytwenty/releases/tag/v0.1.0):

- `.deb` (Debian/Ubuntu): `sudo dpkg -i TwentyTwenty_0.1.0_amd64.deb`
- `.rpm` (Fedora/openSUSE): `sudo rpm -i TwentyTwenty-0.1.0-1.x86_64.rpm`
- `.AppImage` (any distro): `chmod +x TwentyTwenty_0.1.0_amd64.AppImage && ./TwentyTwenty_0.1.0_amd64.AppImage`

No bypass is needed on Linux.

### Windows

Download `TwentyTwenty_0.1.0_x64-setup.exe` (or `TwentyTwenty_0.1.0_x64_en-US.msi`) and run it.
Windows SmartScreen will say "Windows protected your PC." Click **More
info**, then **Run anyway**. This appears because the installer isn't signed
with a paid Microsoft code-signing certificate, not because of anything
found in the binary.

**This platform has never actually been installed or run by a human.** See
Verification status below before relying on it.

### macOS

Download `TwentyTwenty_0.1.0_universal.dmg`, open it, and drag TwentyTwenty
to Applications. Gatekeeper will refuse to open it ("cannot be opened
because the developer cannot be verified"). Clear the quarantine flag from
a terminal:

```sh
xattr -dr com.apple.quarantine /Applications/TwentyTwenty.app
```

Then launch it normally. Again, this is the unsigned-build bypass, not a
statement about the binary's safety.

**This platform has never actually been installed or run by a human.** See
Verification status below before relying on it.

## Verification status

This is the single most important section in this document, and it's tiered
on purpose.

**Linux: genuinely verified.** The idle and inhibition sensing was spike-tested
against a live KDE Plasma 6.7.4 / Wayland session (see
`docs/findings/2026-09-10-linux-sensing.md`), including a real 89-second
passive-video-viewing test that confirmed the "watching counts as screen
time" behavior actually works. The fullscreen multi-window overlay
(`docs/findings/2026-09-10-wayland-overlay.md`) was likewise spike-tested and
then confirmed end to end by a human on that same machine: overlay appears,
stays on top, typing holds the countdown, it completes and auto-closes,
Escape and Skip both dismiss it, and Quit from the tray actually exits the
process. Lock detection was observed transitioning from `false` to `true` on
a real lock event on this machine.

**Windows and macOS: compile and pass their unit tests in CI, nothing more.**
GitHub Actions builds and tests both platforms on every push, and the
installers you can download were produced by that same CI. But the sensing
code itself (`GetLastInputInfo`/`SHQueryUserNotificationState` on Windows,
`CGEventSourceSecondsSinceLastEventType`/`pmset -g assertions` on macOS) has
never run on a real Windows or macOS machine, because none was available
during development. No human has ever installed or launched either build.
They may simply work; they may not. Anyone running one of these two builds
is the first real-world test of that platform's sensing.

## Known limitations

- **Background audio counts as screen time.** The Linux inhibition signal
  KDE reports for a playing video is indistinguishable from the one it
  reports for background music: both were observed carrying the identical
  policy bitmask, and during the actual passing video test, the only reason
  string present the whole time was `"Playing audio"`, not anything
  video-specific. Two candidate filters (the bitmask, and the reason string)
  were both tested against real data and both failed to distinguish audio
  from video, and filtering on either would have broken the passing
  "watching a video counts" case. So leaving music or a podcast playing
  while you walk away will keep the counter running. This is a deliberate,
  evidence-based tradeoff, not an oversight.
- **Multi-monitor placement is unverified.** The overlay creates one
  fullscreen window per connected monitor, but the only machine this project
  was developed and tested on has a single built-in display. The author
  docks to external monitors regularly, which makes this the configuration
  that matters most in practice, and the one nobody has actually watched
  work. See the docked-monitor check in `docs/smoke-checklist.md`.
- **The app shows up in the KDE taskbar** despite requesting otherwise. The
  "skip taskbar" hint is implemented on Linux via GTK's X11-only
  `set_skip_taskbar_hint`/`set_skip_pager_hint` calls; GTK's Wayland backend
  silently no-ops them, and Wayland's core protocol has no client-side
  equivalent at all (hiding a window from the taskbar is deliberately the
  compositor's decision, not an app's). Not fixable from this codebase.
- **macOS lock detection is not implemented**, and `locked` is hardcoded to
  `false` on that platform. Real lock detection would require either a
  private, undocumented API (`CGSSessionScreenIsLocked`) or IOKit FFI, and
  shipping unverifiable private-API code for a platform nobody on this
  project can run or debug was judged not worth the risk: a silently wrong
  `true` would permanently stop the app from ever accumulating screen time,
  with no way to notice. A related, genuinely open question: it's unknown
  whether a locked Mac still reports the display-sleep assertion a video
  call holds. If it does, a locked Mac with an active call could be counted
  as active the whole time it's locked. A Mac owner can settle this by
  locking the screen while a video call or screen share is running and
  comparing `pmset -g assertions` immediately before and after locking.
- **If the overlay's page script fails to load**, the overlay still renders
  (the fade-in is a pure CSS animation with no script dependency) and still
  closes itself automatically when the break timer completes, so it can
  never trap you indefinitely, but the Escape and Skip shortcuts won't work
  for that particular break, since both are wired up in JavaScript.

## Development setup

Built with [Tauri v2](https://tauri.app): a Rust core plus a small plain
HTML/CSS/TypeScript overlay, no UI framework.

### System packages (Arch Linux)

```sh
sudo pacman -S --needed webkit2gtk-4.1 base-devel curl wget file openssl \
  appmenu-gtk-module libappindicator-gtk3 librsvg xdotool
```

(These are Tauri's standard Linux prerequisites; the equivalent Debian/Ubuntu
package names, used by this project's own CI, are
`libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf libxdo-dev`.)

### Rust

Install via the official installer, not a distro package, so it lands
user-locally in `~/.cargo` with no elevated privileges needed:

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

### Node and pnpm

Install pnpm via its own standalone installer. **Corepack is not bundled
with every Node build** (it wasn't on the one used for this project), so
don't rely on `corepack enable`:

```sh
curl -fsSL https://get.pnpm.io/install.sh | sh -
```

Then, from the repo root:

```sh
pnpm install
```

### Running the tests

The engine and probe-parsing tests are pure and run on any platform:

```sh
cargo test --manifest-path src-tauri/Cargo.toml
```

35 tests pass on all three platforms in CI (engine, probe-parsing, and, on
Linux only, since they need a real session bus, two D-Bus integration
tests).

Do not run `pnpm tauri dev` casually: it puts a real fullscreen overlay on
top of whatever you're doing the moment a break interval elapses. If you
need to exercise the overlay by hand, temporarily lower
`WORK_INTERVAL_SECS` in `src-tauri/src/config.rs` first so you're not
waiting 20 minutes for it, and see `docs/smoke-checklist.md` for what to
actually check.

## Project layout

```
src-tauri/src/
  probe/
    mod.rs        ActivityProbe trait -> Sample, plus platform selection
    linux.rs       ext-idle-notify-v1 + PolicyAgent D-Bus + logind
    windows.rs     GetLastInputInfo + SHQueryUserNotificationState
    macos.rs       CGEventSource + pmset assertions
    fallback.rs    input-idle only, used if a platform probe can't start
  engine.rs        the pure accumulator state machine (no I/O, unit tested)
  config.rs        every duration constant in the app
  app.rs           tray, sensing loop, autostart, event dispatch
  overlay.rs       the per-monitor overlay windows
  lib.rs           wiring
public/overlay.html the break screen (plain HTML/CSS/JS, no framework)
docs/
  findings/        the two spike reports this design is built on
  smoke-checklist.md  manual checks to run before tagging a release
```

## Design and further reading

- `docs/superpowers/specs/2026-09-10-twentytwenty-design.md`: the full
  design spec and its rationale, including why Tauri was chosen and what
  was explicitly ruled out of scope.
- `docs/findings/2026-09-10-linux-sensing.md`: the sensing spike, including
  the raw evidence for the passive-viewing catch and the audio/video
  bitmask investigation.
- `docs/findings/2026-09-10-wayland-overlay.md`: the overlay spike that
  decided the fullscreen-per-monitor design, including the multi-monitor
  caveat above.
