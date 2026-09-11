//! macOS `ActivityProbe`.
//!
//! This implementation is unverified (compiled only, never run on any machine
//! involved in this project). Built against the `core-graphics` crate for idle
//! time and `pmset` for display assertion state.
//!
//! Idle time is read from `CGEventSourceSecondsSinceLastEventType`, which
//! returns a float in seconds (already the correct unit, unlike Windows which
//! returns milliseconds). The `core-graphics` crate (0.24) does not wrap this
//! particular C function, so it is declared here directly against Apple's
//! `CGEventSource.h`; everything else it touches (`CGEventSourceStateID`,
//! `CGEventType`) does come from the crate's safe, `#[repr(C)]`/`#[repr(u32)]`
//! types, so only the one function call is `unsafe`.
//!
//! Display-awake and presentation states are read by
//! shelling out to `pmset -g assertions` and caching for five seconds. This is
//! a deliberate trade of elegance for legibility: IOKit-based `IOPMAssertion`
//! queries would require FFI we cannot run and verify, and getting that
//! subtly wrong would be worse than parsing text. The cache keeps the cost
//! reasonable at this polling cadence.
//!
//! **On the `locked` field:**
//!
//! Screen-lock detection on macOS would require either private CGSession APIs
//! (`CGSessionCopyCurrentDictionary` and its undocumented `CGSSessionScreenIsLocked`
//! key) or low-level IOKit interfaces. The `core-graphics` crate exposes only
//! public APIs and does not include CGSession bindings. Using private APIs for
//! a platform we cannot observe or debug would be fragile and create a risk of
//! shipping silently broken behavior (a wrong `true` would prevent the engine
//! from ever accumulating screen time, undetectable without a Mac).
//!
//! Whether a locked macOS session preserves active display assertions is unknown.
//! Assertions are process-scoped, so an app holding `PreventUserIdleDisplaySleep`
//! before the lock has no obvious reason to release it after. This is the same
//! false-positive class that motivated wiring `LockedHint` on Linux. A future
//! maintainer with a Mac could settle this by locking the screen while a video
//! call or screen share is active and comparing `pmset -g assertions` before
//! and after. Until then, `locked` is hardcoded `false`. This differs from Linux,
//! which wires `LockedHint` because a standards-based, testable signal exists
//! there (`logind` DBus API). On macOS, no equivalent cheap, safe, verified
//! alternative is available.

use super::{parse_pmset_assertions, ActivityProbe, Sample};
use core_graphics::event::CGEventType;
use core_graphics::event_source::CGEventSourceStateID;
use std::process::Command;
use std::time::{Duration, Instant};
use log;

const ASSERTION_POLL: Duration = Duration::from_secs(5);

// `core-graphics` 0.24 exposes `CGEventSourceStateID` and `CGEventType` (both
// `#[repr(C)]`/`#[repr(u32)]`, matching the C ABI) but does not wrap
// `CGEventSourceSecondsSinceLastEventType` itself, so it is declared here
// directly against Apple's `CGEventSource.h`:
//   `double CGEventSourceSecondsSinceLastEventType(CGEventSourceStateID, CGEventType)`
#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventSourceSecondsSinceLastEventType(
        state_id: CGEventSourceStateID,
        event_type: CGEventType,
    ) -> f64;
}

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
    /// A failed invocation degrades to (false, false) and is logged.
    fn assertions(&mut self) -> (bool, bool) {
        if let Some((at, held, presenting)) = self.cached_assertions {
            if at.elapsed() < ASSERTION_POLL {
                return (held, presenting);
            }
        }

        let (held, presenting) = match Command::new("pmset")
            .args(["-g", "assertions"])
            .output()
        {
            Ok(output) => {
                if !output.status.success() {
                    log::warn!("pmset -g assertions exited with status {}", output.status);
                    (false, false)
                } else {
                    match String::from_utf8(output.stdout) {
                        Ok(text) => parse_pmset_assertions(&text),
                        Err(e) => {
                            log::warn!("pmset -g assertions stdout was not valid UTF-8: {}", e);
                            (false, false)
                        }
                    }
                }
            }
            Err(e) => {
                // Log every failure even though this runs every 5 seconds. A broken
                // pmset is a platform-level failure that makes screen time tracking
                // non-functional. Repeated warnings in the log are acceptable and
                // expected until the root cause is fixed; they signal to an operator
                // that something is seriously wrong and needs investigation.
                log::warn!("pmset -g assertions failed to spawn: {}", e);
                (false, false)
            }
        };

        self.cached_assertions = Some((Instant::now(), held, presenting));
        (held, presenting)
    }
}

impl ActivityProbe for MacosProbe {
    fn sample(&mut self) -> anyhow::Result<Sample> {
        // Safety: `CGEventSourceSecondsSinceLastEventType` is a pure query with
        // no preconditions beyond linking against the CoreGraphics framework
        // (declared above); it takes plain-old-data arguments and returns a
        // `double`, no pointers or lifetimes involved.
        let idle = unsafe {
            CGEventSourceSecondsSinceLastEventType(
                CGEventSourceStateID::CombinedSessionState,
                CGEventType::Null,
            )
        };
        let (display_held_awake, presenting) = self.assertions();
        Ok(Sample {
            idle_seconds: idle as u64,
            display_held_awake,
            presenting,
            // See module docs for why CGSession lock APIs are not used.
            locked: false,
        })
    }

    fn name(&self) -> &'static str {
        "macos"
    }
}
