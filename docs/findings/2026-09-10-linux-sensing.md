# Linux sensing spike findings (2026-09-10)

Machine: KDE Plasma 6.7.4 / kwin 6.7.4 on Wayland. `DISPLAY=:0` exists via
XWayland but was not used (an X11 idle API would only see XWayland clients
and report nonsense on this session). Single built-in display eDP-1,
1920x1200.

Spike code: `src-tauri/examples/sense_spike.rs` (kept in the working tree,
uncommitted, for the human test below) and
`src-tauri/examples/run_human_test.sh` (the prepared human-run command).
Both are throwaway and should be deleted once the human test is done and
Task 9 starts.

## Bottom line for Task 9

**`org.freedesktop.ScreenSaver.GetSessionIdleTime` does not exist on this
session.** It is present in the DBus introspection XML but calling it
returns a hard DBus error. Do not build Task 9 around it. The working idle
source on this machine is the Wayland-native `ext_idle_notifier_v1`
protocol, used directly via `wayland-client`, not DBus at all.

## Step 1: DBus survey (raw findings)

`busctl --user list | grep -iE 'screensaver|powermanagement|solid|idle'`:

```
org.freedesktop.PowerManagement                1413 org_kde_powerde rouben :1.31 user@1000.service - -
org.freedesktop.PowerManagement.Inhibit        1413 org_kde_powerde rouben :1.31 user@1000.service - -
org.freedesktop.ScreenSaver                    1231 kwin_wayland    rouben :1.7  user@1000.service - -
org.kde.Solid.PowerManagement                  1413 org_kde_powerde rouben :1.31 user@1000.service - -
org.kde.Solid.PowerManagement.PolicyAgent      1413 org_kde_powerde rouben :1.31 user@1000.service - -
org.kde.screensaver                            1231 kwin_wayland    rouben :1.7  user@1000.service - -
```

`org.freedesktop.ScreenSaver` is owned by **kwin_wayland** itself (not a
separate screensaver daemon). `org.kde.Solid.PowerManagement.PolicyAgent`
is owned by **powerdevil** (`org_kde_powerde`).

`busctl --user introspect org.freedesktop.ScreenSaver /ScreenSaver`:

```
org.freedesktop.ScreenSaver interface -  -            -
.GetActive                  method    -  b            -
.GetActiveTime              method    -  u            -
.GetSessionIdleTime         method    -  u            -
.Inhibit                    method    ss u            -
.Lock                       method    -  -            -
.SetActive                  method    b  b            -
.SimulateUserActivity       method    -  -            -
.Throttle                   method    ss u            -
.UnInhibit                  method    u  -            -
.UnThrottle                 method    u  -            -
.ActiveChanged               signal   b  -            -
```

All three candidate methods (`GetActive`, `GetActiveTime`,
`GetSessionIdleTime`) exist per introspection. **Only two of the three
actually work** -- see Step 3.

`busctl --user introspect org.kde.Solid.PowerManagement.PolicyAgent /org/kde/Solid/PowerManagement/PolicyAgent`:

```
org.kde.Solid.PowerManagement.PolicyAgent interface -        -            -
.AddInhibition                            method    uss      u            -
.HasInhibition                            method    u        b            -
.ListInhibitions                          method    -        a{ss}        deprecated
.ReleaseInhibition                        method    u        -            -
.SetInhibitionAllowed                     method    ssb      -            -
.ActiveInhibitions                        property  a(ssssu) 0            emits-change
.RequestedInhibitions                     property  a(ssssu) 0            emits-change
.InhibitionsChanged                       signal    a{ss}as  -            deprecated
```

`ListInhibitions` exists but is flagged `deprecated`, and its declared
signature (`a{ss}`) turned out to be **wrong** -- see Step 3. The
non-deprecated replacement is the `ActiveInhibitions` property, signature
`a(ssssu)` (array of 5-tuples: almost certainly
`(cookie_app, cookie_id, application_name, reason, type_or_flags)` --
not decoded in this spike; worth using instead of `ListInhibitions` in
Task 9 since it is the maintained API and a property read/watch is cheaper
than polling a deprecated method).

`org.freedesktop.login1` fallback (`loginctl show-session 2 -p IdleHint -p IdleSinceHint -p IdleSinceHintMonotonic -p CanIdle`):

```
IdleHint=no
IdleSinceHint=0
IdleSinceHintMonotonic=0
CanIdle=yes
```

## Step 2 & 3: probing example and real-world checks

`src-tauri/Cargo.toml` now has, in addition to what the brief specified:

```toml
zbus = { version = "5", features = ["blocking"] }
anyhow = "1.0.104"
wayland-client = "0.31"
wayland-protocols = { version = "0.32", features = ["client", "staging"] }
```

The last two (`wayland-client`, `wayland-protocols`) are **not** in the
brief -- they were required because no DBus-only approach produced a working
idle value (see below). This is a deviation from "the zbus and anyhow
dependencies stay; Task 9 needs them" in the brief, which assumed
`GetSessionIdleTime` would work. Task 9 will need a Wayland-protocol
dependency too, or an equivalent, unless it finds another way.

### `GetSessionIdleTime`: does not work

Confirmed two ways:

```
$ busctl --user call org.freedesktop.ScreenSaver /ScreenSaver org.freedesktop.ScreenSaver GetSessionIdleTime
Call failed: GetSessionIdleTime is not supported on this platform
```

The zbus-based call in the first version of the spike returned the same
thing structurally:

```
idle=Err(MethodError(OwnedErrorName("org.freedesktop.DBus.Error.NotSupported"),
  Some("GetSessionIdleTime is not supported on this platform"), ...))
```

This is kwin_wayland explicitly refusing the call, not a missing method --
it is present in introspection and answers with a proper DBus error, not
"unknown method." This reads as a deliberate Wayland-session decision by
KDE (X11 idle-time queries are a known fingerprinting/privacy leak vector;
kwin's Wayland backend does not implement this part of the interface).

### `GetActiveTime` and `GetActive`: exist, but do not measure idle time

Polled every 2s for 20s while not touching the machine:

```
14:07:45 activeTime=u 138523  active=b true  idleHint=no 0
14:07:47 activeTime=u 140534  active=b true  idleHint=no 0
14:07:49 activeTime=u 142545  active=b true  idleHint=no 0
...
14:08:03 activeTime=u 156623  active=b true  idleHint=no 0
```

`GetActiveTime` climbs by ~2011ms every 2 real seconds **regardless of
whether the machine is touched** -- it is a free-running counter (almost
certainly "time since the ScreenSaver interface/session started", i.e.
uptime-ish), not idle time. `GetActive` was `true` for the entire window
with no lock/screensaver engaged. Neither is usable as an idle signal on
this session. (`GetActiveTime`'s unit looks like milliseconds given the
~2011/2000ms ratio, but it is moot since it doesn't measure idleness.)

### `logind` `IdleHint`/`IdleSinceHint`: static, does not update on its own

`IdleHint` stayed `no` and `IdleSinceHintMonotonic` stayed `0` for the
entire 20s no-touch window above. Nothing in this session is calling
`Session.SetIdleHint` automatically -- that call is the compositor's/DE's
responsibility, and this Plasma/kwin Wayland session apparently isn't
making it (at least not on the timescale tested). Not usable as a fallback
here without also confirming, separately, whether it ever updates over a
much longer window (not tested -- out of scope for this spike).

### `ListInhibitions`: exists, but its introspected signature is wrong

```
$ busctl --user call org.kde.Solid.PowerManagement.PolicyAgent \
    /org/kde/Solid/PowerManagement/PolicyAgent \
    org.kde.Solid.PowerManagement.PolicyAgent ListInhibitions
aas 0
```

Actual wire signature is **`aas`** (array of arrays-of-string, i.e. a list
of `[key, value]` pairs), not the `a{ss}` (string->string dict) that the
method's own introspection XML claims. Deserializing as
`HashMap<String, String>` in zbus fails with a `SignatureMismatch` error
reporting the real wire type; deserializing as `Vec<Vec<String>>` works.
Result was `aas 0` (empty, zero entries) at the time of testing, which is
correct -- nothing was inhibiting anything, no video was playing.

### Wayland idle protocols: what the compositor actually advertises

`wayland-info` shows the compositor advertising:

```
interface: 'org_kde_kwin_idle',            version:  1, name: 19
interface: 'zwp_idle_inhibit_manager_v1',  version:  1, name: 20
interface: 'ext_idle_notifier_v1',         version:  2, name: 21
```

- `ext_idle_notifier_v1` is the cross-compositor standard (staging)
  protocol and is what the working spike below uses.
- `org_kde_kwin_idle` is KDE's older, KDE-only equivalent; not used here
  since the standard protocol is available and preferred for portability.
- `zwp_idle_inhibit_manager_v1` is a **separate, client-side** protocol
  apps use to say "don't let the compositor idle while this surface is
  visible" (e.g. a video player marking its fullscreen surface). This is
  a strong candidate for how a browser actually prevents idling during
  video playback on Wayland, and it is **not** the same channel as KDE's
  `Solid.PowerManagement.PolicyAgent` DBus inhibitions -- Task 9 may need
  to watch for inhibiting-surface effects (idle simply not advancing)
  rather than assume every real-world inhibitor shows up in
  `ListInhibitions`/`ActiveInhibitions`. This spike did not build a
  zwp_idle_inhibit-side prober; it is a note for Task 9, not a confirmed
  finding.

### Working idle prober: `ext_idle_notifier_v1` directly (not DBus)

Built with `wayland-client` 0.31 + `wayland-protocols` 0.32
(`client`, `staging` features). Design used in
`src-tauri/examples/sense_spike.rs`:

- Bind a `wl_seat` and the `ext_idle_notifier_v1` global.
- Call `get_idle_notification(threshold_ms=1000, seat)` **once**; the
  resulting object fires `Idled` after `threshold_ms` of no input, and
  `Resumed` on the next input, repeatedly, for the object's whole
  lifetime (no need to recreate it after each cycle).
- On `Idled`: record `idled_at = now()`, `is_idle = true`.
- On `Resumed`: `is_idle = false`.
- Reported `idle_ms` = `0` while not idle, else
  `(now() - idled_at) + threshold_ms` while idle (the `+threshold_ms`
  accounts for the fact the compositor only fires `Idled` after that much
  time has already elapsed).

Ran hands-off for 24 real seconds, sampling every 2s:

```
t=  0.0s idle_ms=0     inhibitions=Ok({})
t=  2.0s idle_ms=2255  inhibitions=Ok({})
t=  4.0s idle_ms=4257  inhibitions=Ok({})
t=  6.0s idle_ms=6258  inhibitions=Ok({})
t=  8.0s idle_ms=8259  inhibitions=Ok({})
t= 10.0s idle_ms=10260 inhibitions=Ok({})
t= 12.0s idle_ms=12261 inhibitions=Ok({})
t= 14.0s idle_ms=14262 inhibitions=Ok({})
t= 16.0s idle_ms=16263 inhibitions=Ok({})
t= 18.0s idle_ms=18265 inhibitions=Ok({})
t= 20.0s idle_ms=20267 inhibitions=Ok({})
t= 22.0s idle_ms=22269 inhibitions=Ok({})
t= 24.0s idle_ms=24271 inhibitions=Ok({})
```

`idle_ms` climbs 1:1 with wall-clock time (~2001-2002ms gained per 2s
tick), confirming both that the mechanism works and that **the unit is
milliseconds**, established by direct observation against a wall clock,
not assumed.

**Not independently tested by the agent:** idle resetting to near-zero on
real keyboard/mouse input. The agent has no way to generate genuine input
events on this Wayland session (bash commands do not touch input
devices), and the brief's "idle resets" check (Step 3, item 2) requires
real hardware input. This relies on the `Resumed` event firing as the
protocol specifies; it was not directly observed in this spike. If the
human running the passive-viewing test below also, incidentally, moves
the mouse at the very end, that would be a second confirming data point,
but it isn't required.

## PENDING HUMAN TEST: the video-playing check (Step 3, item 3)

**This is the single most consequential measurement in this project and
has NOT been performed.** No inference has been made about it. It is not
assumed that any inhibition mechanism observed elsewhere applies to video
playback -- this must be measured directly.

Specifically pending:
- Does an inhibition appear in `ListInhibitions` while a fullscreen
  YouTube video plays, naming the browser?
- What is the literal reason string?
- Does `idle_ms` (from `ext_idle_notifier_v1`) keep climbing while the
  video plays (meaning the compositor-level idle notifier is blind to the
  video, and inhibition-watching is the *only* signal that distinguishes
  "video playing" from "actually away"), or does it stay near zero
  (meaning something is resetting/suppressing the idle clock directly)?

**Result: PENDING HUMAN TEST.**

### The exact command for the human to run

```
bash /home/rouben/twentytwenty/src-tauri/examples/run_human_test.sh
```

This script (already built in release mode, so it starts instantly):
- Prints an instruction banner: start a fullscreen video, then do not
  touch the keyboard or mouse.
- Waits 5 seconds, then runs `sense_spike` for exactly 120 seconds via
  `timeout 120s`, so it cannot be left as a stuck process.
- Prints one line every 2 seconds with elapsed time, `idle_ms`, and the
  full inhibitions map.
- Tees all output to `/tmp/tt-sense.log`, in addition to the terminal.

Once that log exists, this section should be updated with the actual
reason string, the actual signature returned, and whether `idle_ms` kept
climbing during playback -- replacing "PENDING HUMAN TEST" with the real
answer.

## Recommendations for Task 9

1. Do not use `org.freedesktop.ScreenSaver.GetSessionIdleTime`,
   `GetActiveTime`, or `GetActive` -- none produce a usable idle signal on
   this session.
2. Do not trust `logind` `IdleHint` as an automatic idle source here
   without further, longer-duration verification -- it did not move in
   the tested window.
3. Use `ext_idle_notifier_v1` (via `wayland-client` +
   `wayland-protocols`, `staging` feature) directly for idle detection.
   Unit: **milliseconds**, and it must be computed client-side from the
   `Idled`/`Resumed` event timestamps -- there is no polled
   "GetIdleTime"-style call in this protocol.
4. For inhibition detection, prefer the non-deprecated
   `ActiveInhibitions` property (`a(ssssu)`) over `ListInhibitions`
   (`aas`, deprecated, and its own introspection signature is wrong).
   Whichever is used, deserialize defensively (as this spike had to) --
   do not trust the DBus introspection XML's declared signature without
   checking a live call.
5. Treat `zwp_idle_inhibit_manager_v1` (client-side inhibit-while-visible)
   as a real possibility for how video players actually prevent idling on
   Wayland, separate from the KDE PolicyAgent DBus inhibition list. If the
   pending human test shows no PolicyAgent inhibition appears during video
   playback, this is the next thing to check, not evidence that "detecting
   video playback" is impossible.
6. The human test result above is the deciding evidence for whether
   inhibition-watching or something else is needed to distinguish "video
   playing" from "user genuinely away." Do not proceed with Task 9's core
   design until that section says something other than PENDING.
