# The overlay input trap (2026-09-11)

Reported from the field:

> when it makes me take a break or even on manual mode on my laptop it
> completely locks my screen, makes it unusable, I have to restart my laptop.
> Esc doesn't close it out, I don't see an overlay, and on the top left corner
> it says connection to localhost refused.

Two independent defects, at two layers. Either one alone is bad. Together they
produce an unrecoverable machine.

## Defect 1: the overlay loaded `devUrl` in a shipped binary

`overlay::show` opens the break window with
`WebviewUrl::App("overlay.html".into())`. How that resolves is decided by a
`cfg` flag, not by the cargo profile (tauri 2.11.5,
`src/manager/mod.rs::get_app_url`):

```rust
#[cfg(dev)]
let url = self.config.build.dev_url.as_ref();
#[cfg(not(dev))]
let url = match self.config.build.frontend_dist.as_ref() { ... };
```

`cfg(dev)` is emitted by `tauri-build` whenever the `tauri` crate is compiled
**without its `custom-protocol` feature** (`tauri-build-2.6.3/src/lib.rs:519`,
`cfg_alias("dev", is_dev())`, reading `DEP_TAURI_DEV` from the `tauri` crate's
own build script). That is the whole test. `--release` has nothing to do with
it.

So a binary built with a bare `cargo build --release`:

- does not embed the frontend at all, and
- points `WebviewUrl::App` at `build.devUrl`, `http://localhost:1420`.

Under `tauri dev` that is correct, because Vite is serving there. In a shipped
binary nothing is listening, so the break overlay is a fullscreen,
always-on-top, decorationless, click-swallowing window displaying WebKit's
"connection refused" page. No script runs, which means the Esc handler and the
Snooze and Skip buttons, all of which live in `public/overlay.html`'s inline
module, **do not exist**. Every dismissal path is gone at once.

Confirmed directly in the build output of the installed binary:

```
$ grep '^cargo:dev' target/release/build/tauri-*/output
cargo:dev=true      (all four)
$ grep 'rustc-cfg=dev' target/release/build/twentytwenty-*/output
cargo:rustc-cfg=dev (all five)
```

`pnpm tauri build` does pass the feature. Verified, not assumed:

```
$ pnpm tauri build --verbose --no-bundle
Running [tauri_cli] Command `cargo build --bins --features tauri/custom-protocol --release`
```

Note it passes `tauri/custom-protocol`, the dependency's feature, so a
`[features]` section in our own `Cargo.toml` is not required and its absence is
not the bug. The bug was building by hand with `cargo build --release` and
copying the result over an installed app.

**Fix:** a `compile_error!` in `lib.rs` under `#[cfg(all(not(debug_assertions),
dev))]`. The exact build that caused this now fails to compile instead of
producing a silent trap. `cargo test` is unaffected (it is a debug build), so
CI still runs.

### A dead end worth recording

`strings` on the binary finds `tt-overlay-` but not `overlay.html`, which reads
as "the asset is missing". It means nothing. With `opt-level = 3` the compiler
loads short string literals as immediate values, splitting them across
instructions: at file offset 4586275 the bytes read `...overlay.I..A.G.html...`,
the literal cut into an 8-byte and a 4-byte `mov`. Do not conclude anything
about this binary from `strings`; read the build script output instead.

## Defect 2: the break hold had no ceiling

This is why the machine needed a restart rather than righting itself in 20
seconds.

```rust
State::OnBreak => {
    if s.idle_seconds >= BREAK_INPUT_GRACE_SECS {
        self.break_remaining = self.break_remaining.saturating_sub(step);
    }
```

The countdown only advances once input has stopped for `BREAK_INPUT_GRACE_SECS`
(2s). That rule is correct on its own: typing through a break is not resting
your eyes, and crediting it would make the app a masked timer, which is the one
thing this project exists not to be.

But the hold was unbounded. Someone whose screen has just been taken over
mashes keys and clicks, which is precisely the input that pins
`break_remaining` at 20 forever. The overlay never counts down, `HideOverlay`
is never emitted, and the window stays up until the process dies. The
instinctive reaction to the trap is also the thing that maintains it.

This defect is independent of defect 1. It bites a fully working overlay too:
anything resting on the keyboard holds the break open indefinitely.

**Fix:** `BREAK_ON_SCREEN_CEILING_SECS` (90s), counted unconditionally from the
moment the overlay appears. Past it the break ends regardless of input. An
ordinary break (finish your sentence, look away) ends on `BREAK_LENGTH_SECS`
and never approaches the ceiling; the regression test
`the_ceiling_does_not_shorten_an_ordinary_break` pins that.

No integrity is lost by letting a determined user out early: Skip already
exists and does the same thing in one click.

## What made this ship

Nothing in the test suite could have caught defect 1, because it is not a
property of the code. It is a property of the build command, and the app under
test in CI is a debug build where `cfg(dev)` is correct. The guard closes that
by making the property a compile-time one.

Defect 2 was reachable by the existing pure-engine tests and simply had no test
asserting it. `every_user_event_that_ends_a_break_hides_the_overlay` covers the
user-event exits; nothing covered "no user event at all, forever".

---

# Round 2: the same trap, a different cause (same day)

The fixes above were shipped as v0.1.3 and the user hit the trap again within
the hour: *"i did not see an overlay... and it still locked me out."*

The first write-up was correct but incomplete. It identified one way the page
can fail to appear and treated fixing that as fixing the trap. It is not the
same thing.

## Why the v0.1.3 verification was worthless

The overlay was verified by building a variant with `decorations(false)`,
`always_on_top(true)` and `set_fullscreen(true)` removed, so it could be
screenshotted without hijacking the tester's screen, and by running the binary
**directly**. Both choices removed the variable that mattered.

The user launches through the AppImage. `AppRun` does two things a direct run
does not: it exports `GDK_BACKEND=x11`, and `AppRun.wrapped` prepends the
bundle's own library directories to `LD_LIBRARY_PATH`. Against a host driver
that disagrees with those bundled libraries, WebKit dies before its first
frame:

```
Could not create default EGL display: EGL_BAD_PARAMETER. Aborting...
```

Measured, four ways:

| launch | result |
|---|---|
| binary directly, native Wayland | renders |
| binary directly, `GDK_BACKEND=x11` | renders |
| via `AppRun` | `EGL_BAD_PARAMETER`, blank window |
| via `AppRun` + `WEBKIT_DISABLE_DMABUF_RENDERER` / `WEBKIT_DISABLE_COMPOSITING_MODE` / `GDK_BACKEND=wayland` | still blank |

No environment variable fixes it. The windowed reproduction is unambiguous: a
titled 720x620 window with nothing inside it, the desktop readable straight
through.

So the same user-visible trap had two unrelated causes, one week's worth of
distinct mechanism apart: the page not loading, and the page loading into a
renderer that cannot paint. Fixing the first did nothing for the second, and
there is no reason to believe a third does not exist.

## The actual fix: stop fixing causes

The overlay no longer gets to capture input on the strength of having been
constructed. It is built **inert** -- `ignore_cursor_events(true)`, unfocused,
not always-on-top -- and is promoted to a real overlay only when the page emits
`tt://overlay-ready`, which it sends after two `requestAnimationFrame` ticks,
i.e. only once a frame has genuinely been composited. No signal within
`OVERLAY_READY_TIMEOUT_MS` (2.5s) and the windows are destroyed and the break
degrades to a notification that says why.

This is cause-agnostic. It covers the dev-URL bug, the EGL bug, and whatever
the third one turns out to be, because it stops asking *why* the page failed
and asks only whether it painted.

### The deadlock that the obvious version walks into

The first attempt created the window with `.visible(false)` and showed it on
ready. That fails 100% of the time, including on healthy machines: an unmapped
window never composites, so `requestAnimationFrame` never fires, so the page
can never report that it painted. Caught only by testing the *success* path --
the log read `overlay did not report a rendered frame` on a machine whose
overlay had rendered perfectly minutes earlier. Inert-but-visible is what
squares it: the window has to be on screen to prove itself, so make it
harmless instead of hiding it.

## Process failures worth naming

1. **Verified the fix on a shape that could not exhibit the bug.** The three
   flags removed to make testing safe were precisely the three that turn a
   blank window into a trap, and the direct launch skipped the environment
   where the failure lives.
2. **Ran the broken build on the user's machine repeatedly while debugging**,
   locking their session several times. The reproduction should have been
   bounded by an external watchdog from the first run, not the fourth.
3. **Shipped a kill switch that did not work.** It ran
   `pkill -f 'lib/twentytwenty/usr/bin/twentytwenty'`, but `AppRun` execs the
   binary with `argv[0]` of `twentytwenty`, so the pattern matched nothing.
   The same wrong pattern was in the test harness cleanup, which is why an
   instance survived and kept trapping the user between tests. `pkill -x
   twentytwenty` is correct.
4. **Told the user to escape via a TTY** without checking they knew their
   account password. They did not. The escape hatch has to work for the person
   who has it, which now means a compositor-level shortcut and an automatic
   timeout, not a shell.

The standing escape is now a KWin script (`~/.local/share/kwin/scripts/ttkillswitch`):
`Ctrl+Alt+K` closes any TwentyTwenty window, and any such window still up after
60 seconds is closed automatically. It lives in the compositor, which owns
input and dispatches global shortcuts before any client sees them, so it cannot
be blocked by the window it is removing -- unlike anything inside the app.
