# TwentyTwenty Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a cross-platform desktop app that enforces the 20-20-20 eye strain rule by measuring genuine screen time rather than wall-clock time.

**Architecture:** A Tauri v2 app whose platform-specific sensing is confined to one small file per operating system, all reducing to a four-field `Sample`. A pure, I/O-free state machine consumes `Sample` values and emits `Command` values, which makes twenty-minute behavior verifiable in microseconds. The tray process holds no webview; overlay windows are built on demand and destroyed after each break.

**Tech Stack:** Rust, Tauri v2, zbus (Linux DBus), windows-rs, core-graphics, vanilla TypeScript + Vite, GitHub Actions.

**Spec:** `docs/superpowers/specs/2026-09-10-twentytwenty-design.md`

## Global Constraints

- App id is `dev.rouben.twentytwenty`. Product name is `TwentyTwenty`.
- Zero monetary cost. No paid code signing, no paid services, no paid data.
- No settings UI, no statistics, no database, no backend, no telemetry, no webcam.
- All intervals live in `src-tauri/src/config.rs` as constants. Nothing else may hardcode a duration.
- `engine.rs` must contain no I/O, no clock access, and no Tauri types. It receives time as `u64` milliseconds.
- Any probe error, or an idle value that moves backwards, permanently downgrades that session to `FallbackProbe` with a logged warning. Never propagate a bad `Sample`.
- Linux is the only verified platform. Windows and macOS code must compile in CI but is shipped as unverified best-effort.
- Repo `RoubenGh/twentytwenty`, public.

## Deviations from the spec

One deliberate simplification, adopted here and worth knowing when reading section 6.2 of the spec:

- The spec lists `DEFER_RECHECK` at 30 seconds. Because the engine already ticks every second, deferral is re-evaluated every tick, which strictly subsumes a 30-second re-check. `DEFER_RECHECK` is therefore **not** implemented as a constant. `DEFER_LIMIT` is unaffected.

---

### Task 1: Toolchain and Tauri scaffold

**Files:**
- Create: `src-tauri/` (generated), `package.json`, `index.html`, `vite.config.ts`, `src/main.ts`
- Create: `.gitignore` (already present, extend)

**Interfaces:**
- Consumes: nothing
- Produces: a runnable `pnpm tauri dev` app; the `src-tauri/src/lib.rs` entry point every later task builds on

- [ ] **Step 1: Install system prerequisites**

```bash
sudo pacman -S --needed rustup base-devel curl wget file openssl \
  libappindicator-gtk3 librsvg github-cli
rustup default stable
```

`webkit2gtk-4.1` is already installed on this machine. Verify with `pacman -Q webkit2gtk-4.1`.

- [ ] **Step 2: Enable a package manager**

Node v26 is installed but `npm` is not on PATH. Use corepack, which ships with Node:

```bash
corepack enable
corepack prepare pnpm@latest --activate
pnpm --version
```

- [ ] **Step 3: Scaffold the Tauri app**

Run from `~/twentytwenty` (the directory already exists and is a git repo, so scaffold in place):

```bash
pnpm create tauri-app@latest . --template vanilla-ts --manager pnpm \
  --identifier dev.rouben.twentytwenty --name twentytwenty
pnpm install
```

If the scaffolder refuses a non-empty directory, generate into `/tmp/tt-scaffold` and copy everything except `.git` across.

- [ ] **Step 4: Verify it runs on KDE Wayland**

Run: `pnpm tauri dev`
Expected: a window opens showing the default template. Close it. If the build fails on a missing system library, install it and note it in `README.md` under prerequisites.

- [ ] **Step 5: Set the product name**

In `src-tauri/tauri.conf.json`, set `"productName": "TwentyTwenty"` and `"identifier": "dev.rouben.twentytwenty"`, and set `app.windows` to an empty array `[]`. The app must not open a window at startup; it is a tray application and all windows are created on demand.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "chore: scaffold Tauri v2 app"
```

---

### Task 2: SPIKE, retire the Wayland overlay risk

This task is an **investigation**, not TDD. Its deliverable is documented evidence plus a decision. It exists because section 9 of the spec identifies compositor behavior as the project's principal risk, and everything in Task 13 assumes an answer.

**Files:**
- Modify: `src-tauri/src/lib.rs`
- Create: `docs/findings/2026-09-10-wayland-overlay.md`

**Interfaces:**
- Consumes: Task 1's scaffold
- Produces: a documented decision, `FULLSCREEN_PER_MONITOR` or `CENTERED_FALLBACK`, consumed by Task 13

- [ ] **Step 1: Write a throwaway overlay spawner**

Replace the body of the `run()` function in `src-tauri/src/lib.rs`:

```rust
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let monitors = app.available_monitors()?;
            println!("monitors found: {}", monitors.len());
            for (i, m) in monitors.iter().enumerate() {
                let pos = m.position();
                let size = m.size();
                println!("  [{i}] pos={pos:?} size={size:?} scale={}", m.scale_factor());
                let w = WebviewWindowBuilder::new(
                    app,
                    format!("spike-{i}"),
                    WebviewUrl::App("index.html".into()),
                )
                .title("TwentyTwenty spike")
                .decorations(false)
                .always_on_top(true)
                .skip_taskbar(true)
                .transparent(true)
                .position(pos.x as f64, pos.y as f64)
                .inner_size(size.width as f64, size.height as f64)
                .build()?;
                w.set_fullscreen(true)?;
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

Add `"withGlobalTauri": true` is not needed. Ensure `src-tauri/tauri.conf.json` has `app.macOSPrivateApi: true` only if transparency misbehaves on macOS; it is irrelevant here.

- [ ] **Step 2: Run it and observe**

Run: `pnpm tauri dev`

Record answers to each of these, by observation, not assumption:

1. How many monitors does `available_monitors()` report, and do the reported positions match your actual layout?
2. Does a window appear on every monitor, or only the primary?
3. Does each window genuinely cover its whole monitor, including over the KDE panel?
4. Does it stay above other windows when you click another application?
5. Does transparency work, or is the background opaque black?
6. Does `Escape` reach the page, and does the window close cleanly?

- [ ] **Step 3: Write up the findings**

Create `docs/findings/2026-09-10-wayland-overlay.md` containing the six answers verbatim, the KDE Plasma and KWin versions (`plasmashell --version`, `kwin_wayland --version`), and a one-line decision:

- If windows land correctly on all monitors and stay on top: **decision is `FULLSCREEN_PER_MONITOR`**.
- If Wayland ignores positioning or only the primary monitor works: **decision is `CENTERED_FALLBACK`**, meaning Task 13 builds a single large centered always-on-top window instead of a per-monitor wash.

Record whichever is true. A negative result here is a success for this task.

- [ ] **Step 4: Revert the spike code**

```bash
git checkout src-tauri/src/lib.rs
```

The findings document is the deliverable. The spike code is not kept.

- [ ] **Step 5: Commit**

```bash
git add docs/findings/2026-09-10-wayland-overlay.md
git commit -m "docs: record Wayland overlay spike findings"
```

---

### Task 3: SPIKE, confirm the Linux sensing sources

Also an investigation. Task 9 implements the Linux probe, and it must be built on APIs confirmed to exist on this machine rather than on assumptions about KDE.

**Files:**
- Create: `src-tauri/examples/sense_spike.rs`
- Create: `docs/findings/2026-09-10-linux-sensing.md`

**Interfaces:**
- Consumes: Task 1's scaffold
- Produces: the confirmed DBus service, object path and method names used by Task 9

- [ ] **Step 1: Survey what the session actually exposes**

```bash
busctl --user list | grep -iE 'screensaver|powermanagement|solid|idle'
busctl --user introspect org.freedesktop.ScreenSaver /ScreenSaver | head -40
busctl --user introspect org.kde.Solid.PowerManagement \
  /org/kde/Solid/PowerManagement/PolicyAgent | head -40
```

Record which services exist and which methods they offer. The candidates are `GetSessionIdleTime`, `GetActiveTime`, `GetActive`, and `ListInhibitions`.

- [ ] **Step 2: Add zbus and write a probing example**

```bash
cd src-tauri && cargo add zbus@5 --features blocking && cargo add anyhow
```

Create `src-tauri/examples/sense_spike.rs`:

```rust
use std::{thread, time::Duration};
use zbus::blocking::Connection;

fn main() -> anyhow::Result<()> {
    let conn = Connection::session()?;
    loop {
        let idle: Result<u32, _> = conn
            .call_method(
                Some("org.freedesktop.ScreenSaver"),
                "/ScreenSaver",
                Some("org.freedesktop.ScreenSaver"),
                "GetSessionIdleTime",
                &(),
            )
            .and_then(|m| m.body().deserialize::<u32>());

        let inhibitions: Result<Vec<(String, String)>, _> = conn
            .call_method(
                Some("org.kde.Solid.PowerManagement.PolicyAgent"),
                "/org/kde/Solid/PowerManagement/PolicyAgent",
                Some("org.kde.Solid.PowerManagement.PolicyAgent"),
                "ListInhibitions",
                &(),
            )
            .and_then(|m| m.body().deserialize::<Vec<(String, String)>>());

        println!("idle={idle:?} inhibitions={inhibitions:?}");
        thread::sleep(Duration::from_secs(2));
    }
}
```

- [ ] **Step 3: Run three real-world checks**

Run: `cd src-tauri && cargo run --example sense_spike`

1. **Idle grows:** stop touching the machine for 90 seconds. Does the idle value climb past 60000 ms (or 60, note the unit)?
2. **Idle resets:** move the mouse. Does it drop to near zero?
3. **Video detected:** play a YouTube video fullscreen and leave the input alone for 90 seconds. Does an inhibition appear naming the browser, and what is its reason string? Does idle keep climbing while the video plays?

Check 3 is the single most important measurement in this project. It is what separates this app from a dumb timer.

- [ ] **Step 4: Write up the findings**

Create `docs/findings/2026-09-10-linux-sensing.md` recording: the exact service, path, interface and method that returned a working idle value; its unit (milliseconds or seconds); the exact signature `ListInhibitions` returned; and the literal reason string a playing video produced. If `GetSessionIdleTime` does not exist, record which of `GetActiveTime`, logind's `IdleSinceHint`, or the `ext-idle-notify-v1` Wayland protocol you fell back to, and why.

- [ ] **Step 5: Delete the spike and commit the findings**

```bash
rm src-tauri/examples/sense_spike.rs
git add -A
git commit -m "docs: record Linux sensing spike findings"
```

The `zbus` and `anyhow` dependencies stay; Task 9 needs them.

---

### Task 4: Config constants, Sample, and the probe trait

**Files:**
- Create: `src-tauri/src/config.rs`
- Create: `src-tauri/src/probe/mod.rs`
- Create: `src-tauri/src/probe/fallback.rs`
- Modify: `src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: Task 1's scaffold
- Produces: `config::*` constants; `probe::Sample { idle_seconds: u64, display_held_awake: bool, presenting: bool, locked: bool }`; `trait ActivityProbe { fn sample(&mut self) -> anyhow::Result<Sample>; fn name(&self) -> &'static str; }`; `probe::fallback::FallbackProbe`

- [ ] **Step 1: Write the failing test**

Create `src-tauri/src/probe/mod.rs` with only the test module for now:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_defaults_to_fully_idle_and_unlocked() {
        let s = Sample::default();
        assert_eq!(s.idle_seconds, 0);
        assert!(!s.display_held_awake);
        assert!(!s.presenting);
        assert!(!s.locked);
    }

    #[test]
    fn fallback_probe_reports_its_name() {
        let mut p = crate::probe::fallback::FallbackProbe::new();
        assert_eq!(p.name(), "fallback");
        let s = p.sample().expect("fallback probe never fails");
        assert!(!s.display_held_awake, "fallback cannot detect assertions");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test`
Expected: FAIL, `cannot find type Sample in this scope`.

- [ ] **Step 3: Write the implementation**

At the top of `src-tauri/src/probe/mod.rs`:

```rust
pub mod fallback;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Sample {
    /// Seconds since the last keyboard or mouse input.
    pub idle_seconds: u64,
    /// Something is asking the OS to keep the display awake (a video is playing).
    pub display_held_awake: bool,
    /// A fullscreen app, presentation mode, or an active screen capture.
    pub presenting: bool,
    /// The session is locked.
    pub locked: bool,
}

pub trait ActivityProbe: Send {
    fn sample(&mut self) -> anyhow::Result<Sample>;
    fn name(&self) -> &'static str;
}
```

`src-tauri/src/probe/fallback.rs`:

```rust
use super::{ActivityProbe, Sample};

/// Used when a platform probe fails. Reports a permanently non-idle session,
/// which degrades the app to a plain interval timer rather than letting a
/// broken sensor stop breaks from ever firing.
pub struct FallbackProbe;

impl FallbackProbe {
    pub fn new() -> Self {
        Self
    }
}

impl Default for FallbackProbe {
    fn default() -> Self {
        Self::new()
    }
}

impl ActivityProbe for FallbackProbe {
    fn sample(&mut self) -> anyhow::Result<Sample> {
        Ok(Sample {
            idle_seconds: 0,
            display_held_awake: false,
            presenting: false,
            locked: false,
        })
    }

    fn name(&self) -> &'static str {
        "fallback"
    }
}
```

`src-tauri/src/config.rs`:

```rust
//! Every duration in the app. Nothing else may hardcode a time value.

pub const TICK_MS: u64 = 1_000;
pub const WORK_INTERVAL_SECS: u64 = 20 * 60;
pub const BREAK_LENGTH_SECS: u64 = 20;
pub const ACTIVE_GRACE_SECS: u64 = 60;
pub const NATURAL_BREAK_SECS: u64 = 2 * 60;
pub const SNOOZE_LENGTH_SECS: u64 = 5 * 60;
pub const BREAK_INPUT_GRACE_SECS: u64 = 2;
pub const DEFER_LIMIT_SECS: u64 = 10 * 60;
```

Add to `src-tauri/src/lib.rs`:

```rust
pub mod config;
pub mod probe;
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src-tauri && cargo test`
Expected: PASS, 2 tests.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: add config constants, Sample, and ActivityProbe trait"
```

---

### Task 5: Engine, activity rule and accumulation

**Files:**
- Create: `src-tauri/src/engine.rs`
- Modify: `src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: `config::*`, `probe::Sample`
- Produces: `engine::Engine::new()`, `Engine::tick(&mut self, s: Sample, now_ms: u64, wall_ms: u64) -> Vec<Command>`, `Engine::state() -> State`, `Engine::bank_secs() -> u64`, `enum State { Accumulating, BreakDue, OnBreak, Snoozed, Paused }`, `enum Command { ShowOverlay, UpdateOverlay { remaining_secs: u64 }, HideOverlay, Notify { title: String, body: String }, TrayStatus(TrayStatus) }`, `enum TrayStatus { Working { bank_secs: u64 }, Break, Snoozed, Paused }`

- [ ] **Step 1: Write the failing tests**

Create `src-tauri/src/engine.rs` with a test module. These four tests encode the entire distinction between this app and a dumb timer, so write them first and read them carefully.

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::*;

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
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test engine`
Expected: FAIL, `cannot find struct Engine`.

- [ ] **Step 3: Write the implementation**

At the top of `src-tauri/src/engine.rs`:

```rust
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

pub struct Engine {
    state: State,
    bank_secs: u64,
    away_secs: u64,
    snooze_secs: u64,
    break_remaining: u64,
    defer_secs: u64,
    paused_until_ms: Option<u64>,
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
            defer_secs: 0,
            paused_until_ms: None,
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
        let step = self.advance_clock(now_ms, wall_ms);
        let active = !s.locked && (s.idle_seconds < ACTIVE_GRACE_SECS || s.display_held_awake);

        let mut cmds = Vec::new();
        if self.state == State::Accumulating {
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
        }
        cmds.push(Command::TrayStatus(TrayStatus::Working {
            bank_secs: self.bank_secs,
        }));
        cmds
    }

    /// Returns how many seconds to advance. A gap larger than two ticks means
    /// the machine slept or the loop stalled.
    fn advance_clock(&mut self, now_ms: u64, wall_ms: u64) -> u64 {
        let mono = self.last_mono_ms.map(|p| now_ms.saturating_sub(p)).unwrap_or(TICK_MS);
        let wall = self.last_wall_ms.map(|p| wall_ms.saturating_sub(p)).unwrap_or(TICK_MS);
        self.last_mono_ms = Some(now_ms);
        self.last_wall_ms = Some(wall_ms);
        (mono.max(wall) / TICK_MS).max(1)
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}
```

Add `pub mod engine;` to `src-tauri/src/lib.rs`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src-tauri && cargo test engine`
Expected: PASS, 6 tests.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: add engine accumulation and activity rule"
```

---

### Task 6: Engine, break due, deferral, and the countdown

**Files:**
- Modify: `src-tauri/src/engine.rs`

**Interfaces:**
- Consumes: Task 5's `Engine`, `State`, `Command`
- Produces: transitions into `State::BreakDue` and `State::OnBreak`; `Command::ShowOverlay`, `Command::UpdateOverlay`, `Command::HideOverlay`, `Command::Notify`

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `src-tauri/src/engine.rs`:

```rust
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test engine`
Expected: FAIL, the engine never leaves `Accumulating`.

- [ ] **Step 3: Write the implementation**

Replace the body of `tick` in `src-tauri/src/engine.rs`:

```rust
    pub fn tick(&mut self, s: Sample, now_ms: u64, wall_ms: u64) -> Vec<Command> {
        let step = self.advance_clock(now_ms, wall_ms);
        let active = !s.locked && (s.idle_seconds < ACTIVE_GRACE_SECS || s.display_held_awake);
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
                if self.break_remaining == 0 {
                    self.finish_break(&mut cmds);
                } else {
                    cmds.push(Command::UpdateOverlay {
                        remaining_secs: self.break_remaining,
                    });
                }
            }
            State::BreakDue | State::Snoozed | State::Paused => {}
        }

        if self.state == State::BreakDue {
            if s.presenting {
                self.defer_secs += step;
                if self.defer_secs >= DEFER_LIMIT_SECS {
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
                cmds.push(Command::ShowOverlay);
                cmds.push(Command::UpdateOverlay {
                    remaining_secs: BREAK_LENGTH_SECS,
                });
            }
        }

        cmds.push(Command::TrayStatus(self.tray_status()));
        cmds
    }

    fn finish_break(&mut self, cmds: &mut Vec<Command>) {
        self.state = State::Accumulating;
        self.bank_secs = 0;
        self.away_secs = 0;
        self.break_remaining = 0;
        cmds.push(Command::HideOverlay);
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
```

Note the ordering: `Accumulating` may transition into `BreakDue` and the `if self.state == State::BreakDue` block then runs in the same tick, so a break fires on the tick the interval completes rather than one tick later.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src-tauri && cargo test engine`
Expected: PASS, 13 tests.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: add break firing, presentation deferral, and countdown"
```

---

### Task 7: Engine, snooze, skip, pause, and break-now

**Files:**
- Modify: `src-tauri/src/engine.rs`

**Interfaces:**
- Consumes: Task 6's `Engine`
- Produces: `enum UserEvent { Snooze, Skip, BreakNow, Pause { for_ms: Option<u64> }, Resume }` and `Engine::on_user(&mut self, ev: UserEvent, now_ms: u64) -> Vec<Command>`

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module:

```rust
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

    #[test]
    fn break_now_shows_the_overlay_immediately() {
        let mut e = Engine::new();
        let cmds = e.on_user(UserEvent::BreakNow, 0);
        assert!(cmds.contains(&Command::ShowOverlay));
        assert_eq!(e.state(), State::OnBreak);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test engine`
Expected: FAIL, `cannot find enum UserEvent`.

- [ ] **Step 3: Write the implementation**

Add to `src-tauri/src/engine.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserEvent {
    Snooze,
    Skip,
    BreakNow,
    Pause { for_ms: Option<u64> },
    Resume,
}
```

Add these methods to `impl Engine`:

```rust
    pub fn on_user(&mut self, ev: UserEvent, now_ms: u64) -> Vec<Command> {
        let mut cmds = Vec::new();
        match ev {
            UserEvent::Snooze => {
                self.state = State::Snoozed;
                self.snooze_secs = 0;
                self.away_secs = 0;
                self.break_remaining = 0;
                cmds.push(Command::HideOverlay);
            }
            UserEvent::Skip => {
                self.state = State::Accumulating;
                self.bank_secs = 0;
                self.away_secs = 0;
                self.snooze_secs = 0;
                self.break_remaining = 0;
                cmds.push(Command::HideOverlay);
            }
            UserEvent::BreakNow => {
                self.state = State::OnBreak;
                self.break_remaining = BREAK_LENGTH_SECS;
                cmds.push(Command::ShowOverlay);
                cmds.push(Command::UpdateOverlay {
                    remaining_secs: BREAK_LENGTH_SECS,
                });
            }
            UserEvent::Pause { for_ms } => {
                self.state = State::Paused;
                self.bank_secs = 0;
                self.away_secs = 0;
                self.snooze_secs = 0;
                self.break_remaining = 0;
                self.paused_until_ms = for_ms.map(|d| now_ms + d);
                cmds.push(Command::HideOverlay);
            }
            UserEvent::Resume => {
                self.state = State::Accumulating;
                self.paused_until_ms = None;
                self.bank_secs = 0;
                self.away_secs = 0;
            }
        }
        cmds.push(Command::TrayStatus(self.tray_status()));
        cmds
    }
```

In `tick`, replace the `State::BreakDue | State::Snoozed | State::Paused => {}` arm with:

```rust
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
                    if now_ms >= until {
                        self.state = State::Accumulating;
                        self.paused_until_ms = None;
                    }
                }
            }
            State::BreakDue => {}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src-tauri && cargo test engine`
Expected: PASS, 20 tests.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: add snooze, skip, pause, and break-now handling"
```

---

### Task 8: Engine, suspend and resume

**Files:**
- Modify: `src-tauri/src/engine.rs`

**Interfaces:**
- Consumes: Task 7's `Engine`
- Produces: no new public API; corrects `advance_clock` behavior across sleep

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module:

```rust
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test engine`
Expected: FAIL. `sleeping_the_machine_counts_as_a_break_not_screen_time` reports `bank_secs == 4200`, because the gap is currently credited to the bank as active time.

- [ ] **Step 3: Write the implementation**

Change `advance_clock` to report whether a gap occurred, and force inactivity when it did. Replace `advance_clock` and the first two lines of `tick`:

```rust
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
```

And in `tick`:

```rust
        let (step, gapped) = self.advance_clock(now_ms, wall_ms);
        let active = !gapped
            && !s.locked
            && (s.idle_seconds < ACTIVE_GRACE_SECS || s.display_held_awake);
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src-tauri && cargo test engine`
Expected: PASS, 23 tests.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: treat suspend and loop stalls as time away from the screen"
```

---

### Task 9: The Linux probe

**Files:**
- Create: `src-tauri/src/probe/linux.rs`
- Modify: `src-tauri/src/probe/mod.rs`
- Create: `src-tauri/tests/linux_probe.rs`

**Interfaces:**
- Consumes: `ActivityProbe`, `Sample`, and the confirmed DBus surface from Task 3's findings document
- Produces: `probe::linux::LinuxProbe::new() -> anyhow::Result<LinuxProbe>`

**Before writing any code, read `docs/findings/2026-09-10-linux-sensing.md`.** It records the exact service, path, method and units that work on this machine. If the code below disagrees with the findings, the findings win.

- [ ] **Step 1: Write the failing integration test**

Create `src-tauri/tests/linux_probe.rs`:

```rust
#![cfg(target_os = "linux")]

use twentytwenty_lib::probe::{linux::LinuxProbe, ActivityProbe};

/// Requires a real desktop session. Skips itself when run headlessly in CI.
fn session_available() -> bool {
    std::env::var("DBUS_SESSION_BUS_ADDRESS").is_ok()
}

#[test]
fn reports_a_plausible_idle_time() {
    if !session_available() {
        eprintln!("skipping: no session bus");
        return;
    }
    let mut p = LinuxProbe::new().expect("probe constructs in a real session");
    let s = p.sample().expect("probe samples without error");
    assert!(s.idle_seconds < 86_400, "idle time must be plausible, got {}", s.idle_seconds);
    assert_eq!(p.name(), "linux");
}

#[test]
fn idle_time_grows_while_untouched() {
    if !session_available() {
        eprintln!("skipping: no session bus");
        return;
    }
    let mut p = LinuxProbe::new().unwrap();
    let first = p.sample().unwrap().idle_seconds;
    std::thread::sleep(std::time::Duration::from_secs(3));
    let second = p.sample().unwrap().idle_seconds;
    assert!(
        second >= first,
        "idle must not move backwards: {first} then {second}"
    );
}
```

The crate must be importable by name from an integration test. In `src-tauri/Cargo.toml`, confirm the `[lib]` section reads:

```toml
[lib]
name = "twentytwenty_lib"
crate-type = ["staticlib", "cdylib", "rlib"]
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test --test linux_probe`
Expected: FAIL, `could not find linux in probe`.

- [ ] **Step 3: Write the implementation**

Create `src-tauri/src/probe/linux.rs`. Adjust the method names and units to match the findings document:

```rust
use super::{ActivityProbe, Sample};
use anyhow::Context;
use zbus::blocking::Connection;

/// Reasons reported by inhibitors that mean the user is presenting rather than
/// merely watching. Matched case-insensitively as substrings.
const PRESENTING_HINTS: &[&str] = &["presentation", "screen sharing", "screencast", "remote"];

pub struct LinuxProbe {
    conn: Connection,
}

impl LinuxProbe {
    pub fn new() -> anyhow::Result<Self> {
        let conn = Connection::session().context("connecting to the session bus")?;
        let mut p = Self { conn };
        // Fail fast at construction if this session cannot answer.
        p.idle_seconds()?;
        Ok(p)
    }

    fn idle_seconds(&mut self) -> anyhow::Result<u64> {
        let ms: u32 = self
            .conn
            .call_method(
                Some("org.freedesktop.ScreenSaver"),
                "/ScreenSaver",
                Some("org.freedesktop.ScreenSaver"),
                "GetSessionIdleTime",
                &(),
            )?
            .body()
            .deserialize()?;
        Ok(u64::from(ms) / 1000)
    }

    fn inhibitions(&mut self) -> Vec<(String, String)> {
        self.conn
            .call_method(
                Some("org.kde.Solid.PowerManagement.PolicyAgent"),
                "/org/kde/Solid/PowerManagement/PolicyAgent",
                Some("org.kde.Solid.PowerManagement.PolicyAgent"),
                "ListInhibitions",
                &(),
            )
            .ok()
            .and_then(|m| m.body().deserialize::<Vec<(String, String)>>().ok())
            .unwrap_or_default()
    }

    fn locked(&mut self) -> bool {
        self.conn
            .call_method(
                Some("org.freedesktop.ScreenSaver"),
                "/ScreenSaver",
                Some("org.freedesktop.ScreenSaver"),
                "GetActive",
                &(),
            )
            .ok()
            .and_then(|m| m.body().deserialize::<bool>().ok())
            .unwrap_or(false)
    }
}

impl ActivityProbe for LinuxProbe {
    fn sample(&mut self) -> anyhow::Result<Sample> {
        let idle_seconds = self.idle_seconds()?;
        let inhibitions = self.inhibitions();
        let display_held_awake = !inhibitions.is_empty();
        let presenting = inhibitions.iter().any(|(app, reason)| {
            let hay = format!("{app} {reason}").to_lowercase();
            PRESENTING_HINTS.iter().any(|h| hay.contains(h))
        });
        let locked = self.locked();
        Ok(Sample {
            idle_seconds,
            display_held_awake,
            presenting,
            locked,
        })
    }

    fn name(&self) -> &'static str {
        "linux"
    }
}
```

In `src-tauri/src/probe/mod.rs`, add:

```rust
#[cfg(target_os = "linux")]
pub mod linux;
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src-tauri && cargo test --test linux_probe`
Expected: PASS, 2 tests.

- [ ] **Step 5: Verify the passive-viewing case by hand**

This is the behavior the whole design rests on, and no automated test can produce it. Write a temporary example that prints a `Sample` every two seconds, play a fullscreen video, keep your hands off the keyboard for 90 seconds, and confirm `display_held_awake` is `true` while `idle_seconds` climbs past 60. Record the result in `docs/findings/2026-09-10-linux-sensing.md` under a "verified" heading, then delete the example.

If `display_held_awake` is `false` while a video plays, stop and report it. The design's central mechanism does not work on this machine and the fallback needs to be reconsidered before continuing.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat: add the Linux activity probe"
```

---

### Task 10: The Windows probe

Compiled by CI, never run by us. Keep it small and obvious.

**Files:**
- Create: `src-tauri/src/probe/windows.rs`
- Modify: `src-tauri/src/probe/mod.rs`, `src-tauri/Cargo.toml`

**Interfaces:**
- Consumes: `ActivityProbe`, `Sample`
- Produces: `probe::windows::WindowsProbe::new() -> anyhow::Result<WindowsProbe>`

- [ ] **Step 1: Add the dependency**

In `src-tauri/Cargo.toml`:

```toml
[target.'cfg(target_os = "windows")'.dependencies]
windows = { version = "0.58", features = [
  "Win32_Foundation",
  "Win32_System_SystemInformation",
  "Win32_UI_Input_KeyboardAndMouse",
  "Win32_UI_Shell",
  "Win32_System_StationsAndDesktops",
] }
```

- [ ] **Step 2: Write the implementation**

Create `src-tauri/src/probe/windows.rs`:

```rust
use super::{ActivityProbe, Sample};
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::StationsAndDesktops::{OpenInputDesktop, DESKTOP_SWITCHDESKTOP};
use windows::Win32::System::SystemInformation::GetTickCount64;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
use windows::Win32::UI::Shell::{
    SHQueryUserNotificationState, QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN,
};

pub struct WindowsProbe;

impl WindowsProbe {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self)
    }
}

impl ActivityProbe for WindowsProbe {
    fn sample(&mut self) -> anyhow::Result<Sample> {
        let mut info = LASTINPUTINFO {
            cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
            dwTime: 0,
        };
        // SAFETY: info is correctly sized and lives for the duration of the call.
        unsafe { GetLastInputInfo(&mut info) }.ok()?;
        let now = unsafe { GetTickCount64() };
        let idle_seconds = now.saturating_sub(u64::from(info.dwTime)) / 1000;

        // SAFETY: no arguments, returns a plain enum.
        let state = unsafe { SHQueryUserNotificationState() }?;
        let presenting = state == QUNS_PRESENTATION_MODE || state == QUNS_RUNNING_D3D_FULL_SCREEN;
        let display_held_awake = presenting || state == QUNS_BUSY;

        // OpenInputDesktop fails when the session is locked.
        // SAFETY: handle is closed immediately when the call succeeds.
        let locked = unsafe {
            match OpenInputDesktop(Default::default(), false, DESKTOP_SWITCHDESKTOP) {
                Ok(h) => {
                    let _ = CloseHandle(h);
                    false
                }
                Err(_) => true,
            }
        };

        Ok(Sample {
            idle_seconds,
            display_held_awake,
            presenting,
            locked,
        })
    }

    fn name(&self) -> &'static str {
        "windows"
    }
}
```

In `src-tauri/src/probe/mod.rs`:

```rust
#[cfg(target_os = "windows")]
pub mod windows;
```

- [ ] **Step 3: Verify it compiles for Windows**

```bash
rustup target add x86_64-pc-windows-msvc
cd src-tauri && cargo check --target x86_64-pc-windows-msvc
```

Expected: compiles. If the `windows` crate's API has shifted, fix the call sites against the version resolved in `Cargo.lock` rather than downgrading the crate. Cross-checking only type-checks; it does not link, which is sufficient here since CI does the real build.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "feat: add the Windows activity probe (unverified)"
```

---

### Task 11: The macOS probe

Also compiled but never run by us. `IOPMAssertion` via raw IOKit FFI is easy to get subtly wrong and impossible for us to test, so the display-awake signal is read from `pmset`, cached for five seconds. That is a deliberate trade of elegance for legibility on an unverifiable platform.

**Files:**
- Create: `src-tauri/src/probe/macos.rs`
- Modify: `src-tauri/src/probe/mod.rs`, `src-tauri/Cargo.toml`

**Interfaces:**
- Consumes: `ActivityProbe`, `Sample`
- Produces: `probe::macos::MacosProbe::new() -> anyhow::Result<MacosProbe>`

- [ ] **Step 1: Add the dependency**

```toml
[target.'cfg(target_os = "macos")'.dependencies]
core-graphics = "0.24"
```

- [ ] **Step 2: Write the implementation**

Create `src-tauri/src/probe/macos.rs`:

```rust
use super::{ActivityProbe, Sample};
use core_graphics::event::CGEventType;
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use std::process::Command;
use std::time::{Duration, Instant};

const ASSERTION_POLL: Duration = Duration::from_secs(5);

pub struct MacosProbe {
    cached_assertions: Option<(Instant, bool, bool)>,
}

impl MacosProbe {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            cached_assertions: None,
        })
    }

    /// Returns (display_held_awake, presenting). Shelling out to pmset is cheap
    /// at this cadence and far easier to reason about than IOKit FFI we cannot run.
    fn assertions(&mut self) -> (bool, bool) {
        if let Some((at, held, presenting)) = self.cached_assertions {
            if at.elapsed() < ASSERTION_POLL {
                return (held, presenting);
            }
        }
        let out = Command::new("pmset")
            .args(["-g", "assertions"])
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_lowercase())
            .unwrap_or_default();

        let held = out
            .lines()
            .any(|l| l.contains("preventuseridledisplaysleep") && !l.trim_start().starts_with('0'));
        let presenting = out.contains("screen sharing") || out.contains("screencapture");

        self.cached_assertions = Some((Instant::now(), held, presenting));
        (held, presenting)
    }
}

impl ActivityProbe for MacosProbe {
    fn sample(&mut self) -> anyhow::Result<Sample> {
        let idle = CGEventSource::seconds_since_last_event_type(
            CGEventSourceStateID::CombinedSessionState,
            CGEventType::Null,
        );
        let (display_held_awake, presenting) = self.assertions();
        Ok(Sample {
            idle_seconds: idle as u64,
            display_held_awake,
            presenting,
            // Screen lock detection needs private CGSession APIs. A locked Mac
            // reports a climbing idle time with no display assertion, which the
            // engine already treats as time away, so this is safe to leave false.
            locked: false,
        })
    }

    fn name(&self) -> &'static str {
        "macos"
    }
}
```

In `src-tauri/src/probe/mod.rs`:

```rust
#[cfg(target_os = "macos")]
pub mod macos;
```

- [ ] **Step 3: Verify it compiles for macOS**

```bash
rustup target add aarch64-apple-darwin
cd src-tauri && cargo check --target aarch64-apple-darwin
```

Expected: compiles. If linking against Apple frameworks fails locally (it usually does without the SDK), rely on CI in Task 15 and record that in `README.md`. `cargo check` alone should still type-check.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "feat: add the macOS activity probe (unverified)"
```

---

### Task 12: Runtime, probe selection and the tick loop

**Files:**
- Create: `src-tauri/src/app.rs`
- Modify: `src-tauri/src/lib.rs`, `src-tauri/src/probe/mod.rs`

**Interfaces:**
- Consumes: `Engine`, `UserEvent`, `Command`, all probes
- Produces: `probe::select() -> Box<dyn ActivityProbe>`; `app::AppState { engine: Mutex<Engine>, }`; `app::run()`; the tray icon and its menu

- [ ] **Step 1: Write the failing test for probe selection**

Add to the `tests` module in `src-tauri/src/probe/mod.rs`:

```rust
    #[test]
    fn select_always_returns_a_working_probe() {
        let mut p = select();
        let s = p.sample();
        assert!(s.is_ok(), "select must never hand back a probe that errors");
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test probe`
Expected: FAIL, `cannot find function select`.

- [ ] **Step 3: Write probe selection**

Add to `src-tauri/src/probe/mod.rs`:

```rust
/// Returns the best probe this platform can offer, falling back when the
/// platform probe cannot be constructed. Never fails.
pub fn select() -> Box<dyn ActivityProbe> {
    #[cfg(target_os = "linux")]
    {
        match linux::LinuxProbe::new() {
            Ok(p) => return Box::new(p),
            Err(e) => log::warn!("linux probe unavailable, falling back: {e}"),
        }
    }
    #[cfg(target_os = "windows")]
    {
        match windows::WindowsProbe::new() {
            Ok(p) => return Box::new(p),
            Err(e) => log::warn!("windows probe unavailable, falling back: {e}"),
        }
    }
    #[cfg(target_os = "macos")]
    {
        match macos::MacosProbe::new() {
            Ok(p) => return Box::new(p),
            Err(e) => log::warn!("macos probe unavailable, falling back: {e}"),
        }
    }
    Box::new(fallback::FallbackProbe::new())
}
```

Add `log` and `env_logger`:

```bash
cd src-tauri && cargo add log env_logger
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd src-tauri && cargo test probe`
Expected: PASS.

- [ ] **Step 5: Write the runtime**

Create `src-tauri/src/app.rs`:

```rust
use crate::config::TICK_MS;
use crate::engine::{Command as EngineCmd, Engine, TrayStatus, UserEvent};
use crate::probe::{self, ActivityProbe, Sample};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager};

pub struct AppState {
    pub engine: Mutex<Engine>,
}

fn wall_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Runs the sensing loop forever. One probe failure downgrades the rest of the
/// session to the fallback probe rather than killing the loop.
pub fn spawn_loop(handle: AppHandle) {
    std::thread::spawn(move || {
        let mut probe: Box<dyn ActivityProbe> = probe::select();
        log::info!("using probe: {}", probe.name());
        let started = Instant::now();
        let mut last_idle: u64 = 0;

        loop {
            std::thread::sleep(Duration::from_millis(TICK_MS));

            let sample = match probe.sample() {
                Ok(s) if s.idle_seconds + 5 >= last_idle || s.idle_seconds == 0 => s,
                Ok(s) => {
                    log::warn!(
                        "idle moved backwards ({last_idle} to {}), downgrading to fallback",
                        s.idle_seconds
                    );
                    probe = Box::new(crate::probe::fallback::FallbackProbe::new());
                    Sample::default()
                }
                Err(e) => {
                    log::warn!("probe failed, downgrading to fallback: {e}");
                    probe = Box::new(crate::probe::fallback::FallbackProbe::new());
                    Sample::default()
                }
            };
            last_idle = sample.idle_seconds;

            let mono = started.elapsed().as_millis() as u64;
            let cmds = {
                let state = handle.state::<AppState>();
                let mut engine = state.engine.lock().unwrap();
                engine.tick(sample, mono, wall_ms())
            };
            dispatch(&handle, cmds);
        }
    });
}

pub fn dispatch(handle: &AppHandle, cmds: Vec<EngineCmd>) {
    for cmd in cmds {
        match cmd {
            EngineCmd::ShowOverlay => crate::overlay::show(handle),
            EngineCmd::HideOverlay => crate::overlay::hide(handle),
            EngineCmd::UpdateOverlay { remaining_secs } => {
                crate::overlay::update(handle, remaining_secs)
            }
            EngineCmd::Notify { title, body } => crate::overlay::notify(handle, &title, &body),
            EngineCmd::TrayStatus(status) => update_tray(handle, status),
        }
    }
}

fn update_tray(handle: &AppHandle, status: TrayStatus) {
    let tip = match status {
        TrayStatus::Working { bank_secs } => {
            let left = crate::config::WORK_INTERVAL_SECS.saturating_sub(bank_secs);
            format!("TwentyTwenty: {} min until your next break", left / 60 + 1)
        }
        TrayStatus::Break => "TwentyTwenty: look away".to_string(),
        TrayStatus::Snoozed => "TwentyTwenty: snoozed".to_string(),
        TrayStatus::Paused => "TwentyTwenty: paused".to_string(),
    };
    if let Some(tray) = handle.tray_by_id("main") {
        let _ = tray.set_tooltip(Some(&tip));
    }
}

pub fn build_tray(app: &tauri::App) -> tauri::Result<()> {
    let break_now = MenuItem::with_id(app, "break_now", "Take a break now", true, None::<&str>)?;
    let snooze = MenuItem::with_id(app, "snooze", "Snooze 5 minutes", true, None::<&str>)?;
    let pause_hour = MenuItem::with_id(app, "pause_hour", "Pause for 1 hour", true, None::<&str>)?;
    let pause = MenuItem::with_id(app, "pause", "Pause until I resume", true, None::<&str>)?;
    let resume = MenuItem::with_id(app, "resume", "Resume", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[&break_now, &snooze, &pause_hour, &pause, &resume, &quit],
    )?;

    TrayIconBuilder::with_id("main")
        .icon(app.default_window_icon().unwrap().clone())
        .tooltip("TwentyTwenty")
        .menu(&menu)
        .on_menu_event(|handle, event| {
            let ev = match event.id().as_ref() {
                "break_now" => Some(UserEvent::BreakNow),
                "snooze" => Some(UserEvent::Snooze),
                "pause_hour" => Some(UserEvent::Pause {
                    for_ms: Some(60 * 60 * 1000),
                }),
                "pause" => Some(UserEvent::Pause { for_ms: None }),
                "resume" => Some(UserEvent::Resume),
                "quit" => {
                    handle.exit(0);
                    None
                }
                _ => None,
            };
            if let Some(ev) = ev {
                let cmds = {
                    let state = handle.state::<AppState>();
                    let mut engine = state.engine.lock().unwrap();
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map(|d| d.as_millis() as u64)
                        .unwrap_or(0);
                    engine.on_user(ev, now)
                };
                dispatch(handle, cmds);
            }
        })
        .build(app)?;
    Ok(())
}
```

Note: `on_user` receives wall-clock milliseconds while `tick` receives monotonic milliseconds for `now_ms`. Only `Pause { for_ms: Some(..) }` compares them, and it compares wall to wall through `paused_until_ms`. Reconcile this by having `tick` also pass wall time into the pause expiry check. Change `State::Paused` in `engine.rs` to compare against a wall-clock field:

```rust
            State::Paused => {
                if let Some(until) = self.paused_until_ms {
                    if wall_ms >= until {
                        self.state = State::Accumulating;
                        self.paused_until_ms = None;
                    }
                }
            }
```

Then update the Task 7 test `a_timed_pause_expires_on_its_own`, which passes `t` for both clocks, so it continues to pass unchanged. Re-run `cargo test engine` to confirm.

Wire it up in `src-tauri/src/lib.rs`:

```rust
pub mod app;
pub mod config;
pub mod engine;
pub mod overlay;
pub mod probe;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::init();
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|_app, _argv, _cwd| {}))
        .manage(app::AppState {
            engine: std::sync::Mutex::new(engine::Engine::new()),
        })
        .setup(|app| {
            app::build_tray(app)?;
            app::spawn_loop(app.handle().clone());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

```bash
cd src-tauri && cargo add tauri-plugin-single-instance
```

`crate::overlay` does not exist yet, so this will not compile until Task 13. That is expected and is why Task 12 and Task 13 are reviewed together.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat: add probe selection, tick loop, and tray menu"
```

---

### Task 13: The overlay

**Read `docs/findings/2026-09-10-wayland-overlay.md` first.** If its decision is `CENTERED_FALLBACK`, build a single centered 900x600 always-on-top window instead of the per-monitor loop below, and record why in a comment.

**Files:**
- Create: `src-tauri/src/overlay.rs`
- Create: `ui/overlay.html`
- Modify: `src-tauri/tauri.conf.json`

**Interfaces:**
- Consumes: `AppHandle`
- Produces: `overlay::show(&AppHandle)`, `overlay::hide(&AppHandle)`, `overlay::update(&AppHandle, remaining_secs: u64)`, `overlay::notify(&AppHandle, title: &str, body: &str)`

- [ ] **Step 1: Write the overlay page**

Create `ui/overlay.html`:

```html
<!doctype html>
<meta charset="utf-8" />
<title>TwentyTwenty</title>
<style>
  :root { color-scheme: dark; }
  html, body {
    margin: 0; height: 100%; overflow: hidden;
    background: rgba(8, 10, 14, 0.88);
    color: #f4f6fb;
    font: 400 16px/1.5 system-ui, -apple-system, "Segoe UI", sans-serif;
    display: grid; place-items: center;
    opacity: 0; transition: opacity 400ms ease;
    -webkit-user-select: none; user-select: none;
  }
  body.visible { opacity: 1; }
  .wrap { text-align: center; }
  .ring { width: 168px; height: 168px; margin: 0 auto 32px; }
  .ring circle { fill: none; stroke-width: 8; }
  .track { stroke: rgba(255,255,255,0.12); }
  .bar {
    stroke: #6ee7b7; stroke-linecap: round;
    transform: rotate(-90deg); transform-origin: 50% 50%;
    transition: stroke-dashoffset 1s linear;
  }
  .count {
    font-size: 44px; font-weight: 600; fill: #f4f6fb;
    font-variant-numeric: tabular-nums;
  }
  h1 { font-size: 30px; font-weight: 600; margin: 0 0 10px; letter-spacing: -0.01em; }
  p { margin: 0 0 36px; color: rgba(244,246,251,0.62); }
  .held { color: #fbbf24; min-height: 1.5em; margin: -24px 0 24px; font-size: 14px; }
  button {
    font: inherit; color: inherit; cursor: pointer;
    background: rgba(255,255,255,0.08);
    border: 1px solid rgba(255,255,255,0.16);
    border-radius: 10px; padding: 10px 20px; margin: 0 6px;
  }
  button:hover { background: rgba(255,255,255,0.14); }
</style>
<div class="wrap">
  <svg class="ring" viewBox="0 0 120 120">
    <circle class="track" cx="60" cy="60" r="54" />
    <circle class="bar" id="bar" cx="60" cy="60" r="54"
            stroke-dasharray="339.292" stroke-dashoffset="0" />
    <text class="count" id="count" x="60" y="60"
          text-anchor="middle" dominant-baseline="central">20</text>
  </svg>
  <h1>Look 20 feet away</h1>
  <p>Rest your eyes for 20 seconds. Blink a few times.</p>
  <div class="held" id="held"></div>
  <button id="snooze">Snooze 5 min (Esc)</button>
  <button id="skip">Skip</button>
</div>
<script type="module">
  const { listen, emit } = window.__TAURI__.event;
  const CIRC = 339.292;
  const TOTAL = 20;
  const bar = document.getElementById("bar");
  const count = document.getElementById("count");
  const held = document.getElementById("held");
  let last = TOTAL;

  requestAnimationFrame(() => document.body.classList.add("visible"));

  listen("tt://tick", (e) => {
    const remaining = e.payload;
    count.textContent = String(remaining);
    bar.setAttribute("stroke-dashoffset", String(CIRC * (1 - remaining / TOTAL)));
    held.textContent = remaining === last ? "Stop typing to start the countdown" : "";
    last = remaining;
  });

  document.getElementById("snooze").onclick = () => emit("tt://snooze");
  document.getElementById("skip").onclick = () => emit("tt://skip");
  addEventListener("keydown", (e) => {
    if (e.key === "Escape") emit("tt://snooze");
  });
</script>
```

- [ ] **Step 2: Write the overlay module**

Create `src-tauri/src/overlay.rs`:

```rust
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_notification::NotificationExt;

const PREFIX: &str = "tt-overlay-";

pub fn show(handle: &AppHandle) {
    let monitors = match handle.available_monitors() {
        Ok(m) => m,
        Err(e) => {
            log::error!("cannot enumerate monitors: {e}");
            return;
        }
    };
    for (i, m) in monitors.iter().enumerate() {
        let label = format!("{PREFIX}{i}");
        if handle.get_webview_window(&label).is_some() {
            continue;
        }
        let pos = m.position();
        let size = m.size();
        let built = WebviewWindowBuilder::new(
            handle,
            &label,
            WebviewUrl::App("overlay.html".into()),
        )
        .title("TwentyTwenty")
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .transparent(true)
        .focused(i == 0)
        .position(pos.x as f64, pos.y as f64)
        .inner_size(size.width as f64, size.height as f64)
        .build();

        match built {
            Ok(w) => {
                let _ = w.set_fullscreen(true);
            }
            Err(e) => log::error!("cannot build overlay on monitor {i}: {e}"),
        }
    }
}

pub fn hide(handle: &AppHandle) {
    for (label, window) in handle.webview_windows() {
        if label.starts_with(PREFIX) {
            let _ = window.close();
        }
    }
}

pub fn update(handle: &AppHandle, remaining_secs: u64) {
    let _ = handle.emit("tt://tick", remaining_secs);
}

pub fn notify(handle: &AppHandle, title: &str, body: &str) {
    let _ = handle
        .notification()
        .builder()
        .title(title)
        .body(body)
        .show();
}
```

```bash
cd src-tauri && cargo add tauri-plugin-notification
```

Register it in `lib.rs` alongside the single-instance plugin:

```rust
        .plugin(tauri_plugin_notification::init())
```

- [ ] **Step 3: Wire the overlay's buttons back to the engine**

In `src-tauri/src/lib.rs`, inside `setup`, after `build_tray`:

```rust
            let h = app.handle().clone();
            app.listen_any("tt://snooze", move |_| {
                let cmds = {
                    let state = h.state::<app::AppState>();
                    let mut engine = state.engine.lock().unwrap();
                    engine.on_user(engine::UserEvent::Snooze, 0)
                };
                app::dispatch(&h, cmds);
            });
            let h2 = app.handle().clone();
            app.listen_any("tt://skip", move |_| {
                let cmds = {
                    let state = h2.state::<app::AppState>();
                    let mut engine = state.engine.lock().unwrap();
                    engine.on_user(engine::UserEvent::Skip, 0)
                };
                app::dispatch(&h2, cmds);
            });
```

Add `use tauri::Listener;` where needed.

- [ ] **Step 4: Point the frontend build at the overlay page**

In `vite.config.ts`, ensure `ui/overlay.html` is an input:

```ts
import { defineConfig } from "vite";

export default defineConfig({
  build: {
    rollupOptions: {
      input: { overlay: "ui/overlay.html" },
    },
  },
  clearScreen: false,
  server: { port: 1420, strictPort: true },
});
```

In `src-tauri/tauri.conf.json`, confirm `build.frontendDist` points at the Vite output directory and that `app.windows` is `[]`.

- [ ] **Step 5: Verify by hand**

Temporarily set `WORK_INTERVAL_SECS` to `10` and `BREAK_LENGTH_SECS` to `5` in `config.rs`, then run `pnpm tauri dev`.

Confirm, in order: the tray icon appears with no window; after 10 seconds the overlay fades in on every monitor; typing holds the countdown and shows the "stop typing" hint; leaving the keyboard alone runs the countdown to zero and closes the overlay; Escape snoozes; the Skip button closes it; the tray menu's items all work.

Restore the real constants before committing. Confirm the diff contains the original values with `git diff src-tauri/src/config.rs` returning nothing.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat: add the break overlay and wire its actions to the engine"
```

---

### Task 14: Autostart and first-run behavior

**Files:**
- Modify: `src-tauri/src/lib.rs`, `src-tauri/Cargo.toml`

**Interfaces:**
- Consumes: the built app
- Produces: autostart enabled on first run

- [ ] **Step 1: Add the plugin**

```bash
cd src-tauri && cargo add tauri-plugin-autostart
```

- [ ] **Step 2: Enable it at startup**

In `src-tauri/src/lib.rs`:

```rust
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
```

Add to the builder chain:

```rust
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
```

And inside `setup`:

```rust
            let autostart = app.autolaunch();
            if let Ok(false) = autostart.is_enabled() {
                if let Err(e) = autostart.enable() {
                    log::warn!("could not enable autostart: {e}");
                }
            }
```

- [ ] **Step 3: Verify**

Run: `pnpm tauri dev`, then check that a launcher entry was written:

```bash
ls ~/.config/autostart/ | grep -i twenty
```

Expected: a `.desktop` entry exists. A failure here must warn and continue, never crash the app.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "feat: enable autostart on first run"
```

---

### Task 15: CI, releases, and the updater

**Files:**
- Create: `.github/workflows/release.yml`
- Create: `.github/workflows/ci.yml`
- Modify: `src-tauri/tauri.conf.json`

**Interfaces:**
- Consumes: the complete app
- Produces: installers for three platforms attached to a GitHub Release, and a working update channel

- [ ] **Step 1: Create the repository**

```bash
gh auth status || gh auth login
gh repo create RoubenGh/twentytwenty --public --source=. --remote=origin --push
```

- [ ] **Step 2: Write the test workflow**

Create `.github/workflows/ci.yml`:

```yaml
name: ci
on:
  push:
    branches: [main]
  pull_request:

jobs:
  test:
    strategy:
      fail-fast: false
      matrix:
        os: [ubuntu-22.04, windows-latest, macos-latest]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - name: Install Linux dependencies
        if: matrix.os == 'ubuntu-22.04'
        run: |
          sudo apt-get update
          sudo apt-get install -y libwebkit2gtk-4.1-dev libappindicator3-dev \
            librsvg2-dev patchelf libxdo-dev
      - run: cargo test --manifest-path src-tauri/Cargo.toml
```

The Linux probe integration tests skip themselves when `DBUS_SESSION_BUS_ADDRESS` is unset, which is the case on CI runners, so this stays green while still compiling every probe.

- [ ] **Step 3: Generate the updater signing key**

```bash
pnpm tauri signer generate -w ~/.tauri/twentytwenty.key
```

This produces a free minisign keypair, unrelated to paid OS code signing. Add the **public** key to `src-tauri/tauri.conf.json`:

```json
  "plugins": {
    "updater": {
      "active": true,
      "pubkey": "PASTE_THE_PUBLIC_KEY_HERE",
      "endpoints": [
        "https://github.com/RoubenGh/twentytwenty/releases/latest/download/latest.json"
      ]
    }
  }
```

Store the private key and its password as repository secrets:

```bash
gh secret set TAURI_SIGNING_PRIVATE_KEY < ~/.tauri/twentytwenty.key
gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD
```

Never commit the private key. Confirm `~/.tauri/` is outside the repo.

- [ ] **Step 4: Write the release workflow**

Create `.github/workflows/release.yml`:

```yaml
name: release
on:
  push:
    tags: ["v*"]

jobs:
  build:
    strategy:
      fail-fast: false
      matrix:
        include:
          - os: ubuntu-22.04
            args: ""
          - os: windows-latest
            args: ""
          - os: macos-latest
            args: "--target universal-apple-darwin"
    runs-on: ${{ matrix.os }}
    permissions:
      contents: write
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v4
        with:
          node-version: 22
      - uses: pnpm/action-setup@v4
        with:
          version: 9
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.os == 'macos-latest' && 'aarch64-apple-darwin,x86_64-apple-darwin' || '' }}
      - name: Install Linux dependencies
        if: matrix.os == 'ubuntu-22.04'
        run: |
          sudo apt-get update
          sudo apt-get install -y libwebkit2gtk-4.1-dev libappindicator3-dev \
            librsvg2-dev patchelf libxdo-dev
      - run: pnpm install
      - uses: tauri-apps/tauri-action@v0
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
          TAURI_SIGNING_PRIVATE_KEY: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY }}
          TAURI_SIGNING_PRIVATE_KEY_PASSWORD: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY_PASSWORD }}
        with:
          tagName: ${{ github.ref_name }}
          releaseName: "TwentyTwenty ${{ github.ref_name }}"
          releaseBody: "Unsigned builds. See the README for the one-time Windows and macOS bypass."
          releaseDraft: false
          includeUpdaterJson: true
          args: ${{ matrix.args }}
```

`includeUpdaterJson: true` is what publishes `latest.json` as a release asset, which is the endpoint configured in step 3.

- [ ] **Step 5: Cut the first release**

```bash
git push -u origin main
git tag v0.1.0
git push origin v0.1.0
gh run watch
```

Expected: three jobs succeed and a release appears carrying `.deb`, `.AppImage`, `.msi`, `.exe`, `.dmg` and `latest.json`. If a job fails, fix it and re-tag with `v0.1.1` rather than force-moving a tag.

- [ ] **Step 6: Verify the Linux artifact actually installs**

```bash
gh release download v0.1.0 --pattern "*.AppImage" --dir /tmp
chmod +x /tmp/*.AppImage && /tmp/TwentyTwenty*.AppImage
```

Expected: the tray icon appears and the app behaves as it did under `tauri dev`.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "ci: add test and release workflows with the updater"
```

---

### Task 16: README and the manual smoke checklist

**Files:**
- Create: `README.md`
- Create: `docs/smoke-checklist.md`

**Interfaces:**
- Consumes: the shipped app
- Produces: the documentation the portfolio case study will draw on

- [ ] **Step 1: Write the README**

Create `README.md` covering: what the 20-20-20 rule is and why this is not a plain timer; how screen time is actually measured, naming the idle-plus-display-assertion rule; the natural-break reset; install instructions per platform including the one-time SmartScreen ("More info", then "Run anyway") and Gatekeeper (`xattr -dr com.apple.quarantine /Applications/TwentyTwenty.app`) bypasses, with a plain statement that builds are unsigned because signing costs money this project does not spend; a note that Linux is the verified platform and Windows and macOS are best-effort; and development setup, listing the Arch packages from Task 1.

- [ ] **Step 2: Write the smoke checklist**

Create `docs/smoke-checklist.md` as a numbered list to run against a release build before tagging. Each line must be observable, not inferred:

1. Launch: tray icon appears, no window opens.
2. Tooltip counts down the minutes remaining.
3. With `WORK_INTERVAL_SECS` temporarily lowered, the overlay fades in on every monitor and sits above other windows.
4. Typing during the break holds the countdown and shows the "stop typing" hint.
5. Not touching anything runs the countdown to zero, and the overlay closes.
6. Escape snoozes; the overlay returns after the snooze interval of active use.
7. Skip closes the overlay and resets the full interval.
8. Play a fullscreen video, hands off the keyboard: the tooltip keeps counting down.
9. Lock the session for three minutes: on return the tooltip shows a full interval remaining.
10. Suspend the machine and resume: the tooltip shows a full interval remaining.
11. Tray "Pause for 1 hour": the tooltip reads paused and the countdown stops.
12. Quit from the tray: the process exits and no window is left behind.

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "docs: add README and manual smoke checklist"
```

---

## Self-review notes

Checked against the spec:

- Spec sections 5, 6 and 6.4 map to Tasks 4 through 8. Section 7 maps to Tasks 3, 9, 10 and 11. Section 8 maps to Tasks 12, 13 and 14. Section 9's risk is retired by Task 2. Section 10 maps to Task 15. Section 11 maps to Tasks 5 through 9 and Task 16. Section 12 is explicitly out of scope for this plan.
- `DEFER_RECHECK` is the one spec constant deliberately not implemented; the reason is recorded at the top of this plan.
- Task 12 introduces a wall-versus-monotonic inconsistency in the pause expiry and fixes it inline in the same step, keeping the Task 7 test valid.
