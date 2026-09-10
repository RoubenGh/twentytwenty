//! Linux `ActivityProbe`.
//!
//! Built directly against `docs/findings/2026-09-10-linux-sensing.md`, a
//! spike run on this exact machine (KDE Plasma 6.7.4 / kwin 6.7.4, Wayland,
//! single display). That document is the authority for every choice below;
//! see it for the raw evidence. In short:
//!
//! - `org.freedesktop.ScreenSaver.GetSessionIdleTime` returns a DBus
//!   `org.freedesktop.DBus.Error.NotSupported` on this session (kwin_wayland
//!   deliberately does not implement it over Wayland). Do not call it.
//! - `GetActiveTime` is a free-running counter (session uptime), not idle
//!   time. `GetActive` was observed `true` for an entire window with nothing
//!   locked. Neither is an idle or lock signal here.
//! - `logind`'s `IdleHint`/`IdleSinceHint` sat static (`no` / `0`) for a full
//!   20s no-input window; nothing in this session updates it on the tested
//!   timescale.
//! - The idle source that actually works is the Wayland-native
//!   `ext_idle_notifier_v1` protocol, used directly (not via DBus). Its unit
//!   is milliseconds, confirmed by direct observation against a wall clock.
//! - `ListInhibitions` is deprecated and its introspection XML lies about its
//!   wire signature (claims `a{ss}`, actually `aas`). The maintained,
//!   correctly-typed replacement is the `ActiveInhibitions` property
//!   (`a(ssssu)`) on `org.kde.Solid.PowerManagement.PolicyAgent`.
//! - The trailing `u` bitmask on `ActiveInhibitions` does NOT distinguish
//!   audio-only from video inhibitions (both observed as `3` live), and the
//!   reason string is not reliable either (the one passing 89s video test
//!   only ever showed `"Playing audio"`). So: any active inhibition, of any
//!   kind, sets `display_held_awake = true`. Do not filter these. The known
//!   consequence -- background audio while away counts as screen time -- is
//!   an accepted, documented limitation.

use super::{ActivityProbe, Sample};
use anyhow::{Context, Result};
use std::time::Instant;
use wayland_client::{
    globals::{registry_queue_init, GlobalListContents},
    protocol::{wl_registry, wl_seat::WlSeat},
    Connection as WlConnection, Dispatch, EventQueue, QueueHandle,
};
use wayland_protocols::ext::idle_notify::v1::client::{
    ext_idle_notification_v1::{self, ExtIdleNotificationV1},
    ext_idle_notifier_v1::ExtIdleNotifierV1,
};
use zbus::blocking::Connection as DbusConnection;

/// Minimum idle timeout requested from the compositor, in milliseconds (the
/// protocol's unit). The compositor fires `Idled` only after this much time
/// has already elapsed with no input, which is why `idle_seconds` below adds
/// it back in.
const IDLE_THRESHOLD_MS: u32 = 1000;

/// D-Bus service that owns the (deprecated-but-that's-fine, we don't use it)
/// `ListInhibitions` and the maintained `ActiveInhibitions` property.
const POLICY_AGENT_SERVICE: &str = "org.kde.Solid.PowerManagement.PolicyAgent";
const POLICY_AGENT_PATH: &str = "/org/kde/Solid/PowerManagement/PolicyAgent";

/// Reasons that hint at presenting (screen sharing, screencasting, a remote
/// session) rather than merely watching something play. This is a
/// best-effort heuristic, not a reliable signal: Wayland's security model
/// forbids clients from inspecting other windows, so there is no way to
/// directly detect "this app is fullscreen" or "this is a presentation" on
/// this platform. The manual pause is the reliable escape hatch here.
/// Matched case-insensitively as a substring of the inhibition's reason.
const PRESENTING_HINTS: &[&str] = &["presentation", "screen sharing", "screencast", "remote"];

/// One row of the `ActiveInhibitions` property: `(category, app_name,
/// reason, action, policy_bitmask)`. Field order and meaning confirmed live
/// against this machine's PolicyAgent; see the findings doc. The bitmask is
/// deliberately not decoded or filtered on -- see module docs.
type Inhibition = (String, String, String, String, u32);

/// Wayland dispatch target for the idle notification object. Holds only
/// what `Idled`/`Resumed` events need to update.
#[derive(Default)]
struct IdleState {
    is_idle: bool,
    idled_at: Option<Instant>,
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for IdleState {
    fn event(
        _state: &mut Self,
        _proxy: &wl_registry::WlRegistry,
        _event: wl_registry::Event,
        _data: &GlobalListContents,
        _conn: &WlConnection,
        _qhandle: &QueueHandle<Self>,
    ) {
        // Every global this probe needs is bound once at startup from the
        // initial registry snapshot; later add/remove events are not acted
        // on.
    }
}

wayland_client::delegate_noop!(IdleState: ignore WlSeat);
wayland_client::delegate_noop!(IdleState: ignore ExtIdleNotifierV1);

impl Dispatch<ExtIdleNotificationV1, ()> for IdleState {
    fn event(
        state: &mut Self,
        _proxy: &ExtIdleNotificationV1,
        event: ext_idle_notification_v1::Event,
        _data: &(),
        _conn: &WlConnection,
        _qhandle: &QueueHandle<Self>,
    ) {
        match event {
            ext_idle_notification_v1::Event::Idled => {
                state.is_idle = true;
                state.idled_at = Some(Instant::now());
            }
            ext_idle_notification_v1::Event::Resumed => {
                state.is_idle = false;
                state.idled_at = None;
            }
            _ => {}
        }
    }
}

pub struct LinuxProbe {
    dbus: DbusConnection,
    wl_queue: EventQueue<IdleState>,
    idle_state: IdleState,
    // Held for its whole lifetime: dropping it would stop Idled/Resumed
    // delivery. It is never recreated per sample, per the findings doc.
    _idle_notification: ExtIdleNotificationV1,
}

impl LinuxProbe {
    pub fn new() -> Result<Self> {
        let dbus = DbusConnection::session().context("connecting to the D-Bus session bus")?;

        let wl_conn = WlConnection::connect_to_env().context(
            "connecting to the Wayland compositor (is WAYLAND_DISPLAY set for this session?)",
        )?;
        let (globals, mut wl_queue) = registry_queue_init::<IdleState>(&wl_conn)
            .context("fetching the Wayland global registry")?;
        let qh = wl_queue.handle();

        let seat: WlSeat = globals
            .bind(&qh, 1..=1, ())
            .context("binding wl_seat")?;
        let idle_notifier: ExtIdleNotifierV1 = globals
            .bind(&qh, 1..=2, ())
            .context("binding ext_idle_notifier_v1 (compositor does not advertise it?)")?;

        let mut idle_state = IdleState::default();
        let idle_notification =
            idle_notifier.get_idle_notification(IDLE_THRESHOLD_MS, &seat, &qh, ());

        // Make sure the requests above actually reached the compositor
        // before this probe is considered constructed.
        wl_queue
            .roundtrip(&mut idle_state)
            .context("initial Wayland roundtrip")?;

        let mut probe = Self {
            dbus,
            wl_queue,
            idle_state,
            _idle_notification: idle_notification,
        };

        // Fail fast: if either signal can't actually be read on this
        // session, `new()` should error so `probe::select()` can fall back
        // rather than hand back a probe that will silently misbehave later.
        probe.idle_seconds()?;
        probe.active_inhibitions()?;

        Ok(probe)
    }

    /// Polls the Wayland connection for any `Idled`/`Resumed` events that
    /// have arrived since the last call, then derives idle time from
    /// `IdleState`. `idle_ms` is `0` while not idle, else `(now - idled_at)
    /// + threshold`, per the findings doc -- the compositor only fires
    /// `Idled` after `threshold` has already elapsed, so that much must be
    /// added back in.
    fn idle_seconds(&mut self) -> Result<u64> {
        self.wl_queue
            .roundtrip(&mut self.idle_state)
            .context("polling Wayland idle notifications")?;

        let idle_ms: u64 = if self.idle_state.is_idle {
            let elapsed_ms = self
                .idle_state
                .idled_at
                .map(|t| t.elapsed().as_millis() as u64)
                .unwrap_or(0);
            elapsed_ms + u64::from(IDLE_THRESHOLD_MS)
        } else {
            0
        };

        Ok(idle_ms / 1000)
    }

    /// Reads the `ActiveInhibitions` property. Returns `Err` on genuine
    /// D-Bus failure (used by `new()` to fail fast); callers in `sample()`
    /// degrade a read failure to "no inhibitions" instead of propagating it,
    /// since a transient PolicyAgent hiccup should not stop breaks from
    /// firing.
    fn active_inhibitions(&self) -> Result<Vec<Inhibition>> {
        let proxy = zbus::blocking::Proxy::new(
            &self.dbus,
            POLICY_AGENT_SERVICE,
            POLICY_AGENT_PATH,
            POLICY_AGENT_SERVICE,
        )
        .context("building the PolicyAgent proxy")?;
        proxy
            .get_property::<Vec<Inhibition>>("ActiveInhibitions")
            .context("reading ActiveInhibitions")
    }
}

impl ActivityProbe for LinuxProbe {
    fn sample(&mut self) -> Result<Sample> {
        let idle_seconds = self.idle_seconds()?;

        // A read failure here degrades to "nothing is inhibiting" rather
        // than propagating: a missing/misbehaving PolicyAgent should not
        // stop breaks from firing, it should just lose inhibition-awareness
        // for that one sample.
        let inhibitions = self.active_inhibitions().unwrap_or_default();

        let display_held_awake = !inhibitions.is_empty();
        let presenting = inhibitions.iter().any(|(_category, app, reason, _action, _bits)| {
            let hay = format!("{app} {reason}").to_lowercase();
            PRESENTING_HINTS.iter().any(|hint| hay.contains(hint))
        });

        // No verified lock signal exists on this session. `GetActive`
        // (org.freedesktop.ScreenSaver) was directly observed `true` for an
        // entire window with nothing locked, so it is not usable here. The
        // standards-based alternative, `org.freedesktop.login1` Session's
        // `LockedHint` property, does exist and currently reads correctly
        // (`false` while genuinely unlocked) -- but confirming it actually
        // flips to `true` during a real lock would require locking this
        // live, in-use session with no way for this probe to unlock it
        // again (no input-generation capability), so that was not
        // performed. Per the task ruling, an unverified signal is not
        // wired in: always report `false`. This is safe because a locked
        // session presents as climbing idle_seconds with zero input, which
        // the engine already treats as time away from the screen.
        let locked = false;

        // Implausible values degrade rather than propagate: idle_seconds is
        // a monotonically-derived duration and cannot be negative, but as a
        // last line of defense a nonsensical multi-year value (e.g. a clock
        // glitch) is clamped rather than handed to the engine.
        const IMPLAUSIBLE_IDLE_SECONDS: u64 = 365 * 24 * 60 * 60;
        let idle_seconds = if idle_seconds > IMPLAUSIBLE_IDLE_SECONDS {
            0
        } else {
            idle_seconds
        };

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
