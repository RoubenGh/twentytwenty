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
7. **Spike v2 (round 2)**, to resolve the three questions round 1 couldn't answer: watchdog extended to 45 seconds, window pointed at a purpose-built `public/spike.html` (transparent page body, one clearly-bordered opaque panel reading "SPIKE — click another window, then press Escape", and a keydown handler calling `window.__TAURI__.window.getCurrentWindow().close()` on Escape). Built ahead of time (`cargo build`) and verified `spike.html` was actually served by a standalone `vite` dev server before handoff, but **not launched by the agent** — the user ran `pnpm tauri dev` themselves and drove the interaction (clicking another window, pressing Escape) directly, then reported results and a screenshot. That is the source of the Q4/Q5/Q6 updates below. `src-tauri/src/lib.rs` and `public/spike.html` were reverted/deleted afterward; nothing from spike v2 is kept in the tree.

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
**PASS — observed by the human tester in spike v2, not inferred by the agent.** The user ran the spike themselves, clicked another application window while the overlay was up, and reported the overlay did not go away — it stayed on top. This was the load-bearing open question from round 1 and it resolved positively.

**Q5. Does transparency work, or is the background opaque black?**
**WORKS — confirmed via the user's screenshot evidence from spike v2, overriding their own initial impression.** The user's first-glance read of the overlay was "solid black," but the screenshot they captured during the run shows otherwise: outside the spike's bordered panel, their own browser window (a tab showing "Rankings · Consensus · Splits · Odds · Streaks") and terminal text are visible through the overlay. That is the live desktop showing through a genuinely transparent window — only the bordered panel itself is opaque, and only because `spike.html`'s CSS deliberately paints it that way (`.panel { background: #111318; ... }` against an otherwise `background: transparent` body). Conclusion: **KWin honors `.transparent(true)` on Wayland.** (Note: this reverses the round-1 finding, which was correctly marked inconclusive because the round-1 template page had no transparent region to test against — round 2's purpose-built page fixed that confound.)

**Q6. Does Escape reach the page, and does the window close cleanly?**
Split answer, both halves now resolved. **Escape reaching the page: PASS, observed by the human.** The on-screen status text updated to a specific error message when Escape was pressed, which is direct proof the keydown handler ran inside the webview:
```
Escape received, close() failed: window.close not allowed. Permissions associated with this command: core:window:allow-close
```
**The close itself failed — but purely on a capabilities/permissions ground, not a Wayland/compositor ground.** See "Required capability changes for Task 13" below. Separately, general process/window lifecycle (creation, forced kill, clean exit) was confirmed clean in round 1 by two independent mechanisms (watchdog auto-exit, code 0; manual `pkill -f twentytwenty`, SIGTERM/143), with no hangs, zombies, or stuck windows in either run.

## Required capability changes for Task 13

This is the single most actionable output of the spike. The Escape-to-close failure in Q6 was **not** a Wayland/compositor limitation — it was Tauri's own permission system refusing the call. The exact error, verbatim, as it rendered on screen when the user pressed Escape:

```
Escape received, close() failed: window.close not allowed. Permissions associated with this command: core:window:allow-close
```

Root cause: `src-tauri/capabilities/default.json` grants only `core:default` and `opener:default`, and is scoped with `"windows": ["main"]`. The spike's overlay windows were labelled `spike-0` (and Task 13's real overlay windows are expected to be labelled something like `tt-overlay-N`) — neither matches `"main"`, so those windows receive none of the window-control or event permissions they need, including `core:window:allow-close`.

**Task 13 must, before wiring up any close/dismiss interaction on the overlay:**
- Grant `core:window:allow-close` (and any other window/event permissions the real overlay needs, e.g. show/hide, always-on-top toggling, event listening) to windows matching the overlay's actual label pattern.
- Do this either by widening `default.json`'s `"windows"` list to include the overlay label pattern, or — preferable for least-privilege — by adding a dedicated capability file scoped to just the overlay windows with just the permissions they need.
- Without this, Task 13's overlay will visually behave correctly but Escape/any programmatic close will silently (or loudly, as here) fail at the permissions layer, independent of anything Wayland-related.

## KWin non-interactive introspection

`busctl --user introspect org.kde.KWin /KWin` was used instead of the forbidden interactive `queryWindowInfo`/`showDebugConsole` calls. Finding: KWin's session D-Bus interface exposes no non-interactive "list all windows" method. `queryWindowInfo` requires the user to click a target window to identify it, and `getWindowInfo` needs a UUID obtained that way. This is itself a finding: there is no scriptable way to enumerate/verify overlay window stacking order from outside the compositor without interactive assistance, which is part of why Q4 needs a human.

## Decision

**`FULLSCREEN_PER_MONITOR`**

All criteria the decision hinges on are now satisfied: positioning matches an independent source (`kscreen-doctor`), fullscreen coverage includes the panel, the window lifecycle is clean, transparency works (Q5, confirmed by screenshot evidence), the overlay stays above other windows after a focus change (Q4, confirmed by direct human observation), and Escape reaches the webview (Q6, confirmed by direct human observation) — its close call fails only on a capabilities/permissions ground that Task 13 can and must fix (see "Required capability changes for Task 13" above), not a compositor ground.

**Caveat carried forward, not blocking the decision:** this test machine has a single built-in display, so the "per-monitor" half of `FULLSCREEN_PER_MONITOR` — a correctly-positioned, correctly-fullscreened overlay on *each* of several simultaneous monitors — has not actually been exercised on multiple displays at once. The user has confirmed they regularly dock to external displays, so this path ships **unverified on multi-monitor hardware** until the next time they're docked. This belongs on the **Task 16 smoke checklist** as a docked-only check: confirm `available_monitors()` reports all connected displays with correct positions, and confirm an overlay window lands correctly fullscreened on each one, while docked.
