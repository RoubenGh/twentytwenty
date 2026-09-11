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
**Superseded, see "Update (2026-09-11)" below.** The original answer here was "yes," based on a single screenshot comparison. Two later real-world captures of the shipped overlay, and a follow-up re-test using KWin's own window introspection rather than a screenshot, both contradict it: the panel remains visible, because the overlay window is not actually reaching a genuine fullscreen state at the compositor level on this machine. The screenshot-based method used in round 1 could not detect this, because it only ever compared two images and never asked the compositor what state the window itself was in.

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

**This decision's shape (one borderless, always-on-top, transparent window per monitor) still stands as of the 2026-09-11 update below.** What no longer stands is the specific claim that this window, as currently built, reaches genuine compositor-level fullscreen on this KDE Wayland setup: see below for what was re-tested and what, if anything, would need to change to fix that.

## Update (2026-09-11): panel-coverage re-test

The app owner reported that two full-screen captures of the shipped v0.1.0/v0.1.1 overlay, taken on this same machine, show the KDE panel visible and undimmed at the bottom in both, directly contradicting Q3's original "yes." This was re-investigated rather than taken on faith either way, since the original evidence and the new report can't both be right.

**Screenshots turned out not to be a usable instrument here.** Repeated `spectacle -b -n -f` captures taken while the shipped overlay was on-screen came back as a single uniform color filling the whole frame (solid black, then on a later attempt solid white), with zero texture anywhere, on multiple separate attempts. Whatever this is, it is a capture artifact, not real desktop content, since prior (pre-overlay) and post-overlay captures on the same session came back normal and detailed. Round 1 got a clean, informative screenshot because that spike's window setup happened not to trigger whatever this artifact is; that shouldn't have been read as proof captures are reliable evidence for this specific always-on-top/transparent/skip-taskbar window combination.

**KWin's own window introspection was used instead**, via `org.kde.kwin.Scripting`'s `loadScript`/`run` (non-interactive, unlike `queryWindowInfo`), running a script that calls `workspace.windowList()` and dumps each window's `fullScreen`, `keepAbove`, `skipTaskbar`, and frame geometry to the journal. This is a materially better instrument than a screenshot comparison: it asks the compositor what it believes about the window directly, rather than inferring it from pixels.

The result, reproduced identically across three separate fresh launches of the actual shipped binary (not spike code), clicking "Take a break now" over D-Bus each time: KWin reports the overlay window at roughly 382x169, centered on the 1920x1200 monitor, with `fullScreen: false`, `keepAbove: false`, and `skipTaskbar: false` (an ordinary small floating window, not a fullscreen one, regardless of what was requested).

This is the more surprising part: **Tauri's own state query disagrees with KWin.** Calling `is_fullscreen()`, `outer_size()`, and `outer_position()` on the same `WebviewWindow` object, moments after `build()`, reports `is_fullscreen: true`, `outer_size: 1920x1200`, `outer_position: (0, 0)` (GTK believes the window is exactly right). `is_always_on_top()` is the one property GTK itself admits is wrong, reporting `false` despite `.always_on_top(true)` being requested. So this isn't a case of the app requesting the wrong thing: it's a case of GTK's Wayland backend and KWin ending up with two different beliefs about the same window, and KWin's belief is what actually gets drawn.

One candidate fix was tried and ruled out: requesting fullscreen as a builder-time attribute (`WebviewWindowBuilder::fullscreen(true)`, applied atomically as part of the window's initial attributes) instead of the shipped code's post-build `set_fullscreen(true)` call (sent asynchronously after the window is already up, which was the suspected race, since GTK's fullscreen call can snapshot "the size to restore to" from whatever geometry happens to be current at that moment). This was tested against the same KWin introspection and made no difference at all: identical 382x169 window, identical `fullScreen: false`. Whatever GTK and KWin disagree about here, it isn't simply about request timing, and chasing it further (a `wlr-layer-shell` surface bypassing GTK's toplevel entirely, or a KWin-specific window-rule workaround) is a materially bigger change than this finding's scope, so it was not attempted. The speculative builder-time change was reverted rather than kept, since it demonstrably fixed nothing and would have been an unverified behavior change on Windows and macOS (where "start fullscreen" and "fullscreen after show" are not guaranteed to animate or negotiate identically) for no measured benefit.

**Conclusion:** Q3's original "yes" is wrong for the app as currently shipped. The overlay does not reliably reach a real fullscreen state on this KDE Plasma 6.7.4 / Wayland machine, which is why the KDE panel (and, by the same mechanism, probably any other window) remains visible over it. This is very likely the same root cause as the already-documented "shows up in the KDE taskbar" limitation (see README): both `skip_taskbar` and the fullscreen/always-on-top state are Tauri/GTK window hints that this Wayland backend queues up and reports success for, but that do not reliably reach KWin as the genuine compositor states they're supposed to produce. See the README's "Known limitations" for the user-facing writeup. No code change was made for this finding beyond the reverted experiment described above; `src-tauri/src/overlay.rs` is unchanged from before this update.
