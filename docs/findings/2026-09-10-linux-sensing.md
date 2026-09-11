# Linux sensing spike findings (2026-09-10)

Machine: KDE Plasma 6.7.4 / kwin 6.7.4 on Wayland. `DISPLAY=:0` exists via
XWayland but was not used (an X11 idle API would only see XWayland clients
and report nonsense on this session). Single built-in display eDP-1,
1920x1200.

Spike code (`src-tauri/examples/sense_spike.rs`,
`src-tauri/examples/run_human_test.sh`, and the `wayland-client` /
`wayland-protocols` additions to `src-tauri/Cargo.toml`) was used to
produce the results below and has since been reverted from the working
tree. Task 9 should add its own real implementation and dependencies
rather than resurrect this throwaway code.

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
`a(ssssu)`, confirmed via `gdbus introspect` to carry the Qt type name
`QList<PolicyAgentInhibition>`. Its 5-tuple fields, decoded from live data
(see "The false-positive risk" section below), are
`(category_label, app_name, reason, action, policy_bitmask)` -- NOT the
`(cookie_app, cookie_id, application_name, reason, type_or_flags)` shape
originally guessed here. Use `ActiveInhibitions` in Task 9 instead of
`ListInhibitions`: it is the maintained API and a property read is cheaper
than polling a deprecated method.

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

## Human test: the video-playing check (Step 3, item 3) -- PASS

The human ran `bash src-tauri/examples/run_human_test.sh`, started a
fullscreen video with sound, and left the keyboard/mouse untouched. Output
was captured to `/tmp/tt-sense.log` (118 of the 120 seconds captured before
the harness's own timeout cut the log; the script's own 120s `timeout` was
about to end it anyway, so this is a complete result).

From t=30s to t=118s (89 unbroken seconds), `idle_ms` climbed continuously
from 0 to 88963 with **no resets**, i.e. no keyboard/mouse input was
registered for the entire span. Throughout that same span the inhibitions
map read exactly:

```
Ok({"firefox": "Playing audio"})
```

**Result: PASS.** A plain idle check (no inhibition awareness) would have
declared the user "away" after 60s of no input. The inhibition-aware
design correctly identifies that something is holding the session active
despite zero input, which is exactly the natural-break-suppression
behavior the spec requires. Raw numbers (idle_ms at each 2s tick from
t=30s): climbing steadily from 0 to 88963 over the 88s window, consistent
with `ext_idle_notifier_v1`'s `idle_ms` computation not resetting because
no `Resumed` event fired.

## The false-positive risk: audio-only inhibitions

The reason string captured above is **`"Playing audio"`**, not "playing
video". The product spec's rule is "something is holding the **display**
awake" -- a session/suspend inhibition triggered by background audio (e.g.
someone starts a podcast or Spotify and leaves for lunch with the screen
unattended) is a different thing, and treating it as screen-active would
produce a false positive: the break bank keeps filling for someone who
isn't looking at a screen at all.

### What the 5th field of `ActiveInhibitions` (`a(ssssu)`) encodes

Read live, right now, via both `busctl --user get-property` and
`gdbus call ... Properties.Get`:

```
$ busctl --user get-property org.kde.Solid.PowerManagement.PolicyAgent \
    /org/kde/Solid/PowerManagement/PolicyAgent \
    org.kde.Solid.PowerManagement.PolicyAgent ActiveInhibitions
a(ssssu) 2 "idle" "firefox" "Playing audio" "block" 3 "idle" "firefox" "Playing video" "block" 3
```

At that moment there were **two simultaneous** inhibitions from firefox:
one reasoned `"Playing audio"`, one reasoned `"Playing video"` -- both with
identical trailing values: category `"idle"`, action `"block"`, bitmask
**`3`**. `gdbus introspect` confirms the Qt struct name
`QList<PolicyAgentInhibition>` and that `AddInhibition(in u types, in s
app_name, in s reason, out u cookie)` is the write side, so the `u` in the
tuple is that same `types` bitmask, echoed back.

A few seconds later, polled again, the video-reasoned row had disappeared
and only this remained (also confirmed stable across 8 more polls, ~1.5s
apart):

```
(<[('sleep', 'firefox', 'Playing audio', 'block', uint32 3)]>,)
```

Two things established from this, both directly observed, not guessed:

1. **The bitmask value observed in every sample, for both "Playing audio"
   and "Playing video", was the same: `3`.** Decoding `3` against KDE
   PowerDevil's known `PolicyAgent` `RequiredPolicies` flags
   (`InterruptSession = 1`, `ChangeScreenSettings = 2`, `ChangeProfile =
   4` -- this enum is recalled from general KDE/PowerDevil ecosystem
   knowledge; no header or source for it was found on this machine to
   verify against directly, see below), `3 = InterruptSession |
   ChangeScreenSettings`, i.e. "don't suspend the session" AND "don't
   change screen settings (dim/blank)". **Firefox requested the exact
   same bitmask for its audio-only-reasoned inhibition as for its
   video-reasoned one.** No source/header file for this enum was found on
   this machine (`find` turned up no PolicyAgent headers or XML;
   `libpowerdevilcore.so` is stripped and its string table doesn't contain
   the enum names) -- the bit values above are inferred from the known
   public KDE `PolicyAgent` API rather than confirmed against source
   present on this machine, and should be treated as a strong hypothesis,
   not a certainty.
2. **The first field (`"idle"` vs `"sleep"`) changed on its own, for the
   same ongoing audio inhibition, without any bitmask change.** This
   means the first field is not a stable identifier of inhibition
   category -- it looks like an internal "what's imminently blocked next"
   label that can drift, not something safe to filter on.

### Is the bitmask a reliable audio-vs-video discriminator? NOT ESTABLISHED -- needs a human test

The one clean side-by-side data point available (the audio+video rows
captured simultaneously above) shows **identical bitmasks (3) for both**.
That is real evidence *against* naively filtering `ActiveInhibitions` by
"does the bitmask include the screen-settings bit" as a fix: on this data,
firefox's audio-only-reasoned inhibition already requests the same
screen-related bit as its video-reasoned one. It is possible this was not
actually a clean "audio only, no video visible" scenario (the video from
the passive-viewing test may still have been open/paused nearby, or
Firefox may always request both bits for any tab with a `<video>` element
regardless of whether that element is the one making sound) -- this could
not be confirmed without touching the GUI, which was out of scope for the
agent.

**This needs a targeted human test before Task 9 relies on the bitmask (or
anything else) to distinguish audio-only from video.** No guess is being
recorded in its place.

**Exact steps for a human to run this follow-up:**

1. Close all video tabs/players entirely (nothing with a `<video>`
   element anywhere, even paused/hidden).
2. Start audio-only playback with **no video element on the page at all**
   -- e.g. a music streaming site's audio-only view, a plain `<audio>`
   tag test page, or a local `.mp3` played in a minimal player. Confirm
   with `busctl --user get-property org.kde.Solid.PowerManagement.PolicyAgent /org/kde/Solid/PowerManagement/PolicyAgent org.kde.Solid.PowerManagement.PolicyAgent ActiveInhibitions`
   that only a `"Playing audio"` (or similar) reason shows up, and record
   its 5th-field value.
3. Stop the audio, then start a fullscreen video with sound and record
   `ActiveInhibitions` again while it plays, noting its 5th-field value.
4. Compare the two values. If they differ, the bitmask is a usable
   discriminator and Task 9 should filter on it. If they are identical (as
   the one data point here suggests they might be), the bitmask cannot
   distinguish these cases on this browser/version, and Task 9 will need
   a different approach (see ruling below).

## Ruling for Task 9

1. **Idle source: `ext_idle_notifier_v1`, unit milliseconds.**
   `org.freedesktop.ScreenSaver.GetSessionIdleTime` returns a DBus
   `org.freedesktop.DBus.Error.NotSupported` error on this session --
   confirmed directly via both `busctl --user call` and zbus. Task 9 must
   **not** call it, and must not use `GetActiveTime` or `GetActive`
   either (neither measures idle time -- see Step 3 above). `logind`
   `IdleHint` is not usable as an automatic fallback here either -- it did
   not move in the tested window. The confirmed working source is
   `ext_idle_notifier_v1`, computed client-side from `Idled`/`Resumed`
   event timestamps as `idle_ms = (now - idled_at) + threshold_ms` while
   idle, else `0`. This requires `wayland-client` and `wayland-protocols`
   (`staging` feature) as **required dependencies for Task 9** -- not
   optional, since no DBus-only path produces a working idle value on
   this machine.
2. **Inhibitions: use `ActiveInhibitions`, not `ListInhibitions`.**
   `ActiveInhibitions` (property, `a(ssssu)`, Qt type
   `QList<PolicyAgentInhibition>`) is the maintained, non-deprecated API.
   `ListInhibitions` is deprecated and its own introspection XML lies
   about its wire signature (claims `a{ss}`, actually `aas`) -- deserialize
   defensively regardless of which is used, do not trust the declared
   DBus signature without checking a live call.
3. **Do not treat "any active inhibition" as "display is being held
   awake."** The human test proved inhibition-awareness works (PASS,
   above) but also proved the naive version is unsafe: Firefox's
   audio-only-reasoned inhibition (`"Playing audio"`) carried the
   identical policy bitmask (`3`) as its video-reasoned one in the one
   side-by-side sample captured here. **Do not ship a bitmask filter
   (e.g. "only count it if `ChangeScreenSettings` is set") without first
   running the follow-up human test above** to confirm the bitmask ever
   actually differs between audio-only and video content -- on current
   evidence it may not.
4. **If the follow-up test shows the bitmask never discriminates,**
   Task 9 needs a different signal than `ActiveInhibitions` alone to
   satisfy "holds the *display* awake" specifically -- candidates worth
   investigating next, in order: (a) `zwp_idle_inhibit_manager_v1`, the
   separate client-side Wayland protocol apps use to inhibit idling for a
   specific visible surface (confirmed advertised by the compositor, not
   yet probed -- this is closer to "display" semantics than a DBus
   session-wide inhibition is, since it's tied to a rendered surface); or
   (b) a fragile, explicitly-flagged reason-string heuristic (e.g.
   excluding reasons that look audio-only) as a stopgap, clearly
   documented as app-specific and not guaranteed to generalize beyond
   Firefox's current wording.
5. Do not proceed with Task 9's inhibition-filtering design until the
   follow-up bitmask comparison test (audio-only vs. video, exact steps
   above) has been run and this document updated with its result.
