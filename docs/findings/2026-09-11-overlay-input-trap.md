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
