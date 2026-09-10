# Spike findings: Wayland overlay risk (2026-09-10)

## Environment

- KDE Plasma on Wayland (`$XDG_SESSION_TYPE` = `wayland`; `DISPLAY=:0` also present via XWayland but not used by this spike).
- `plasmashell --version` -> `plasmashell 6.7.4`
- `kwin_wayland --version` -> `kwin 6.7.4`
- Test hardware has a **single physical display** (laptop panel `eDP-1`, 1920x1200). Multi-monitor positioning could not be exercised on this machine; see caveats below.

## What was run

1. `src-tauri/src/lib.rs` was temporarily replaced with the spike code from the task brief (per-monitor fullscreen, borderless, always-on-top, transparent, skip-taskbar window built from `available_monitors()`), plus a mandatory safety watchdog thread that force-exits the process after 20 seconds (`std::process::exit(0)`).
2. `cargo check` was run first to confirm the spike compiled before ever spawning a window.
3. `pnpm tauri dev` was run twice from `/home/rouben/twentytwenty`:
   - Run 1: watched stdout for the `monitors found` line, then let the 20-second watchdog fire on its own. Process exited with code 0; `pgrep -af twentytwenty` showed no leftover processes.
   - Run 2: watched stdout for the `monitors found` line, then immediately captured `spectacle -b -n -f -o /tmp/spike-full.png` (fullscreen capture) while the window was live, then manually ran `pkill -f twentytwenty` well inside the 20-second budget. Process tree (tauri-cli, vite, `target/debug/twentytwenty`) exited cleanly (SIGTERM, exit code 143); `pgrep -af` confirmed nothing left running.
4. A baseline screenshot (`/tmp/spike-baseline.png`) was taken after the spike exited, to check for the presence/location of the KDE panel on the live desktop.
5. `busctl --user introspect org.kde.KWin /KWin` was used (introspection only — not the interactive `queryWindowInfo`/`showDebugConsole` calls) to check for a non-interactive window-list API.
6. `src-tauri/src/lib.rs` was reverted with `git checkout src-tauri/src/lib.rs` once evidence was collected. Screenshots were left in `/tmp`, not committed.

## Raw evidence

`monitors found: 1` printed by the spike, with:
```
[0] pos=PhysicalPosition { x: 0, y: 0 } size=PhysicalSize { width: 1920, height: 1200 } scale=1
```

`kscreen-doctor -o` (independent source, not the app):
```
1 eDP-1 ... enabled, connected, priority 1, Panel
Geometry: 0,0 1920x1200
Scale: 1
```
These match exactly.

`/tmp/spike-full.png` (reviewed with the Read tool): the entire 1920x1200 capture is filled edge-to-edge with the Tauri starter template's "Welcome to Tauri" page (dark background, Vite/Tauri/TS logos, a name input and Greet button). No desktop, dock, panel, or wallpaper is visible anywhere in the frame.

`/tmp/spike-baseline.png` (taken after the spike exited): shows the normal desktop with a bottom-edge panel/taskbar (icons and a system clock) visible. Plasma config confirms a bottom panel exists: `~/.config/plasma-org.kde.plasma.desktop-appletsrc` contains a containment with `plugin=org.kde.panel`, `location=4`. That panel area is fully absent — covered — in the spike screenshot.

`busctl --user introspect org.kde.KWin /KWin` output includes `queryWindowInfo` (interactive: requires the user to click a target window) and `getWindowInfo(s)` (takes a window UUID, which in practice is obtained via the interactive call). No method that lists/enumerates all open windows non-interactively was found on this interface.

## The six questions

**Q1. How many monitors does `available_monitors()` report, and do the reported positions match the actual layout?**
Answered, observed. It reported 1 monitor at position `(0, 0)`, size `1920x1200`, scale `1`. This matches `kscreen-doctor -o` exactly (`eDP-1`, geometry `0,0 1920x1200`). Positions were honored correctly for the one monitor present. **Caveat: this machine has only one physical display, so cross-monitor position agreement (the scenario the risk is actually about) was not exercised.**

**Q2. Does a window appear on every monitor, or only the primary?**
Answered, observed. A window appeared on the (only) monitor, confirmed visually in `/tmp/spike-full.png`. With only one monitor available, "every monitor" and "primary only" are indistinguishable in this test — this specific multi-monitor claim is unverified.

**Q3. Does each window genuinely cover its whole monitor, including over the KDE panel?**
Answered, observed. Yes. The spike screenshot shows the app content filling the entire 1920x1200 frame with no panel visible, while a baseline screenshot taken moments later (spike closed) shows a bottom panel/taskbar present at that location on the same monitor. The overlay covered the panel.

**Q4. Does it stay above other windows when you click another application?**
**NEEDS HUMAN OBSERVATION.** This requires genuine interactive focus-switching (clicking a different app window while the overlay is up) that cannot be simulated or inferred from a static screenshot or process state. Not tested; no claim is made either way.

**Q5. Does transparency work, or is the background opaque black?**
**Inconclusive from available evidence — recorded honestly rather than guessed.** The window was built with `.transparent(true)`, but the Tauri starter template's own page content paints an opaque dark background (`Welcome to Tauri` page) across the entire viewport. A screenshot of "true compositor transparency showing the wallpaper through" and "an opaque dark webview background" would look identical here, because the confound is the app content itself, not the compositor. No genuinely transparent region was visible anywhere in the capture, but this spike cannot distinguish "Wayland ignored `.transparent(true)`" from "the page's own CSS is simply opaque on top of a working transparent window." Answering this for real requires a test page with an explicit `background: transparent` body and a visual check for the desktop showing through — that step was not part of the brief's spike page and was not performed.

**Q6. Does Escape reach the page, and does the window close cleanly?**
Split answer. **Window/process lifecycle: answered, observed** — the process exited cleanly by two independent mechanisms: the 20-second watchdog (`std::process::exit(0)`, exit code 0, no leftover processes per `pgrep -af`) and manual `pkill -f twentytwenty` (SIGTERM, exit code 143, no leftover processes). No hangs, no zombie processes, no stuck windows in either run. **Whether Escape specifically reaches the webview and triggers a close: NEEDS HUMAN OBSERVATION.** The brief's spike code registers no Escape/keyboard handler at all, so this was never wired up or tested; only the external kill paths were exercised.

## KWin non-interactive introspection

`busctl --user introspect org.kde.KWin /KWin` was used instead of the forbidden interactive `queryWindowInfo`/`showDebugConsole` calls. Finding: KWin's session D-Bus interface exposes no non-interactive "list all windows" method. `queryWindowInfo` requires the user to click a target window to identify it, and `getWindowInfo` needs a UUID obtained that way. This is itself a finding: there is no scriptable way to enumerate/verify overlay window stacking order from outside the compositor without interactive assistance, which is part of why Q4 needs a human.

## Decision

**CONDITIONAL.** The criteria this spike could measure directly were all satisfied on the one available display: `available_monitors()` positions agree with an independent source (`kscreen-doctor`), the window's requested position/size were honored, fullscreen mode covered the entire monitor including the panel, and the window lifecycle (creation, forced kill, clean exit) was reliable with no hangs.

What remains open is exactly the part the decision language hinges on ("stay on top" — Q4) and the multi-monitor claim (Q1/Q2, untestable on single-display hardware):

- If a human confirms the overlay **stays above other windows** after clicking a different application (Q4 = yes), and this holds when tested on the actual multi-monitor target hardware: **decision is `FULLSCREEN_PER_MONITOR`.**
- If a human observes that clicking another window **raises it above the overlay**, or that a second monitor does not receive its own correctly-positioned window: **decision is `CENTERED_FALLBACK`** — Task 13 should build a single large centered always-on-top window instead of a per-monitor wash.

No default is assumed; this must be resolved by the human observation on Q4 (and, ideally, a re-run of Q1/Q2 on multi-monitor hardware) before Task 13 starts.
