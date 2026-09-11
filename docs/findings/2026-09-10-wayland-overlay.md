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
**Corrected, see "Update (2026-09-11)" below.** The original answer here was an unqualified "yes," based on the round-1 spike (a simpler window than what actually shipped: no `skip_taskbar`, no `always_on_top`) genuinely covering the panel in that one test. The window that actually shipped in `overlay.rs` combines `skip_taskbar(true)` with `always_on_top(true)` and transparency, and real-world use of that shipped window shows the KDE panel visible and undimmed at the bottom during a break, i.e. the round-1 result did not carry over to the shipped window unchanged. Pixel-level re-measurement (below) confirms the shipped overlay does genuinely cover the full 1920px width of the screen edge to edge with correct compositing; it just doesn't extend into the panel's own strip at the bottom. See the update below for the evidence and the likely mechanism.

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

**This decision's shape (one borderless, always-on-top, transparent window per monitor) still stands as of the 2026-09-11 update below.** What no longer stands is the specific claim that this window covers the KDE panel: see below for the pixel-level re-measurement and the likely mechanism.

## Update (2026-09-11): panel-coverage re-test

The app owner reported that real-world captures of the shipped overlay show the KDE panel visible and undimmed at the bottom, contradicting Q3's original unqualified "yes." This was re-investigated rather than taken on faith either way, since the original evidence and the new report can't both be right as stated.

**First attempt, since abandoned: KWin's own scripting introspection.** `org.kde.kwin.Scripting`'s `loadScript`/`run` (non-interactive, unlike `queryWindowInfo`) was used to run a script calling `workspace.windowList()` and dumping each window's `fullScreen`, `keepAbove`, `skipTaskbar`, and frame geometry to the journal, on the theory that asking the compositor directly beats inferring state from pixels. Reproduced identically across three fresh launches of the actual shipped binary, clicking "Take a break now" over D-Bus each time, it reported the overlay window as roughly 382x169, centered on the 1920x1200 monitor: a small floating window, not a fullscreen one. **This reading is now known to be wrong** (see the pixel evidence below, which directly contradicts a 382x169 centered window) and should not be trusted or repeated as a method for this window type without independent corroboration. It is left in this document as a warning, not as a finding: either `workspace.windowList()` reported stale/cached geometry for this specific always-on-top/transparent/skip-taskbar window combination, or it picked up some secondary/intermediate surface rather than the actual composited toplevel. Which one, if either, is unresolved.

**What actually settled it: pixel measurement against a known backdrop.** A committed screenshot of the live overlay (`Projects/Portfolio/public/images/twentytwenty/overlay.png`, 1920x1160) was taken over a fullscreen code editor whose body color is `#11131a` (17, 19, 26 sRGB) and whose title bar is `#171a22` (23, 26, 34 sRGB). The overlay's own CSS sets the body background to `rgba(8, 10, 14, 0.88)` (`public/overlay.html`). Alpha-compositing that over each backdrop color predicts, per channel, roughly `0.88*8 + 0.12*17 ≈ 9.1` over the editor body and `0.88*8 + 0.12*23 ≈ 9.8` over the title bar (all channels, since the overlay's own tint is nearly neutral).

Four points were measured directly in the committed PNG (verified independently with `magick`/`identify`, not just taken on report):

| point | expected backdrop | measured (sRGB) |
|---|---|---|
| (5, 5) | title bar | 11, 13, 17 |
| (1915, 5) | title bar | 11, 13, 17 |
| (100, 600) | editor body | 10, 13, 16 |
| (1900, 600) | editor body | 10, 12, 16 |

These match the compositing prediction to within about a point (consistent with sRGB gamma rounding), **at both the far-left and far-right edges of the frame**, and the same additionally holds at the bottom row of this capture, (5, 1159) and (1915, 1159), both still at 10-13 across all channels: dimmed, not panel-bright. A 382x169 window centered on a 1920x1200 monitor cannot influence a pixel at x=5 or x=1915 (it would only span roughly x=769-1151), so this alone disproves the KWin introspection reading above. **The overlay genuinely covers the full 1920px width of the screen, edge to edge, with correct alpha compositing over whatever is behind it.**

The captured image is 1920x1160, not 1920x1200: 40px short of the full monitor height. The KDE panel, per the original round-1 evidence in this document, is a `location=4` (bottom) panel roughly 46px tall. **The most likely explanation, consistent with everything measured:** the overlay window is sized to the desktop's work area (the region KWin reserves for normal application windows, i.e. the monitor minus the panel's reserved strip) rather than to the monitor's full physical output. That would produce exactly this signature: correct, edge-to-edge coverage and compositing everywhere the window actually exists, with the panel's own strip simply never overlapped because the window doesn't extend there, rather than the window covering the panel and something rendering through it. It would also plausibly share a root cause with the already-documented "shows up in the KDE taskbar" limitation: both are consistent with KWin (or GTK's Wayland backend) treating this window as an ordinary work-area-constrained application window rather than as an exempted overlay/panel-level surface, for the same underlying reason.

This is the most likely explanation given the evidence, not a confirmed root cause. It has not been verified by inspecting the window's actual geometry against the monitor's work-area geometry (`kscreen-doctor`/`xdg-output` do not expose work-area size directly, and the KWin introspection method that could have checked this is exactly the one just shown to be unreliable here). A future spike wanting to pin this down further would need a trustworthy non-pixel source for the window's live geometry, which this document does not currently have.

**Always-on-top (Q4) is downgraded from settled to unresolved, not disproven.** The round-1 human observation (Q4, above) that the overlay stayed on top after a focus change still stands as direct evidence for the spike's simpler window. But the specific `keepAbove: false` reading for the shipped window came from the same KWin scripting introspection just shown to be unreliable for this window (it also disagreed with Tauri's own `is_always_on_top()`, and disagreed with the now-confirmed-correct pixel evidence on coverage), so that particular reading cannot be relied on as evidence that always-on-top is broken. Whether the shipped window reliably stays above other windows in practice is currently an open question, not an established fact in either direction.

**Conclusion:** Q3's original unqualified "yes" was wrong in one specific respect: the overlay does not cover the KDE panel's own strip at the bottom. It was right in every other respect now checked at the pixel level: the overlay does genuinely, correctly cover the full width and the rest of the screen's height, edge to edge, with correct transparency compositing. The likely mechanism is work-area sizing rather than full-output sizing, which would also explain the taskbar-entry limitation. See the README's "Known limitations" for the user-facing writeup. No code change was made for this finding; `src-tauri/src/overlay.rs` is unchanged. A speculative fix (requesting fullscreen as a builder-time attribute instead of the shipped code's post-build `set_fullscreen(true)` call) was tried during the now-discredited KWin-introspection investigation, showed no measurable difference under that method, and was reverted; it was not re-evaluated against the pixel method and should not be assumed ruled out on that basis alone.
