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
    // System bus, for `org.freedesktop.login1`'s `LockedHint` -- best-effort:
    // its absence degrades `locked()` to `false` rather than failing
    // construction, since idle-tracking and inhibition-awareness (validated
    // below) are the two signals this probe cannot do without.
    system_dbus: Option<DbusConnection>,
    wl_queue: EventQueue<IdleState>,
    idle_state: IdleState,
    // The resolved login1 session object path, cached after the first
    // successful `LockedHint` read. See `login1_locked_hint`.
    session_path: Option<zbus::zvariant::OwnedObjectPath>,
    // Held for its whole lifetime: dropping it would stop Idled/Resumed
    // delivery. It is never recreated per sample, per the findings doc.
    _idle_notification: ExtIdleNotificationV1,
}

impl LinuxProbe {
    pub fn new() -> Result<Self> {
        let dbus = DbusConnection::session().context("connecting to the D-Bus session bus")?;
        let system_dbus = DbusConnection::system().ok();

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
            system_dbus,
            wl_queue,
            idle_state,
            session_path: None,
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

    /// Reads `org.freedesktop.login1` Session `LockedHint` for the current
    /// graphical session, via the system bus. `org.freedesktop.ScreenSaver`
    /// `GetActive` is disqualified as a lock signal on this session (the
    /// findings doc observed it `true` for an entire window with nothing
    /// locked); `LockedHint` is the standards-based alternative.
    ///
    /// Verified live, both directions, on this machine:
    /// - Unlocked: reads `false` (cross-checked against `loginctl
    ///   show-session`).
    /// - Actually locked (the session auto-locked mid-development here,
    ///   confirmed independently by `kscreenlocker_greet` running and
    ///   `loginctl show-session 2 -p LockedHint` also reporting `yes`):
    ///   reads `true`. This is a genuine observed positive transition, not
    ///   an assumption.
    ///
    /// Session resolution deliberately does not rely solely on
    /// `GetSessionByPID` for the calling process's own PID: directly
    /// observed on this machine, a process launched outside
    /// `session-2.scope` (e.g. this probe's own throwaway test binaries,
    /// run from an agent-spawned shell) gets
    /// `org.freedesktop.login1.NoSessionForPID` even though the real KDE
    /// session is alive and correctly reports `LockedHint`. A shipped app
    /// autostarted as a systemd user service hits the same failure for the
    /// same reason (it isn't a member of the session's cgroup either) --
    /// which means the `ListSessions` fallback below is not a rare corner
    /// case, it is the path production actually takes. So: try
    /// `GetSessionByPID` first (the precise, correct answer when it
    /// resolves), and fall back to `ListSessions`, filtered and
    /// disambiguated per the hazard documented on `resolve_login1_session`.
    /// Any failure anywhere in this chain degrades to `Err`, and the caller
    /// in `sample()` folds that to `false` -- the same value this probe
    /// already reported before `LockedHint` was wired in, so there is no
    /// regression risk.
    ///
    /// The resolution above runs ONCE and the winning object path is then
    /// cached for the life of the probe. It used to run on every sample,
    /// which meant a `GetSessionByPID` that production always fails, a
    /// `/proc/self/status` read, a `ListSessions`, and an `Active` read per
    /// candidate -- four or more system-bus round trips every second, to
    /// re-derive a value that changes a few times a day (a login, a fast user
    /// switch). That is not just waste: this probe's `sample()` is what paces
    /// the tick loop, and the engine treats any tick gap over `2 * TICK_MS`
    /// as time spent away from the desk, so bus contention could manufacture
    /// phantom away-time and reset a legitimately accumulated bank. Only a
    /// failed `LockedHint` read invalidates the cache, which is exactly the
    /// signal that the cached session went away (logged out, switched);
    /// resolution is then retried once, immediately.
    fn login1_locked_hint(&mut self) -> Result<bool> {
        // Cloned (it is an Arc handle internally) so that the cache below can
        // be mutated while a connection is in hand.
        let system = self
            .system_dbus
            .as_ref()
            .context("no system D-Bus connection")?
            .clone();

        if let Some(path) = self.session_path.clone() {
            match Self::read_locked_hint(&system, &path) {
                Ok(locked) => return Ok(locked),
                Err(e) => {
                    log::debug!("cached login1 session went stale, re-resolving: {e}");
                    self.session_path = None;
                }
            }
        }

        let manager = zbus::blocking::Proxy::new(
            &system,
            "org.freedesktop.login1",
            "/org/freedesktop/login1",
            "org.freedesktop.login1.Manager",
        )
        .context("building the login1 Manager proxy")?;

        let session_path = Self::resolve_login1_session(&manager)?;
        let locked = Self::read_locked_hint(&system, &session_path)?;
        self.session_path = Some(session_path);
        Ok(locked)
    }

    /// Reads `LockedHint` off one already-resolved login1 session path.
    fn read_locked_hint(
        system: &DbusConnection,
        session_path: &zbus::zvariant::OwnedObjectPath,
    ) -> Result<bool> {
        let session = zbus::blocking::Proxy::new(
            system,
            "org.freedesktop.login1",
            session_path,
            "org.freedesktop.login1.Session",
        )
        .context("building the login1 Session proxy")?;
        session
            .get_property::<bool>("LockedHint")
            .context("reading LockedHint")
    }

    /// Resolves the login1 session object path whose `LockedHint` this
    /// process should read.
    ///
    /// HAZARD, and why the uid filter below is load-bearing, not
    /// decoration: this is the probe's *normal* path in production (see
    /// `login1_locked_hint`'s doc comment -- `GetSessionByPID` fails for
    /// any process outside the session's cgroup, which includes a
    /// systemd-user-service autostart). `ListSessions` returns every
    /// session on the machine, for every user. On a single-user machine the
    /// first seated row happens to be the right one, but on a machine with
    /// two graphical sessions belonging to different users (fast user
    /// switching, or a second seat), picking "the first seated row" can
    /// resolve to a DIFFERENT user's session and read THEIR lock state.
    /// That is worse than reading none: it is confidently wrong, with
    /// nothing downstream able to tell right from wrong. So candidates are
    /// filtered to this process's own uid before anything else -- never
    /// select a session this process's user does not own. If no seated
    /// session for this uid exists, this returns `Err`, which
    /// `login1_locked_hint` degrades to `false`, exactly like every other
    /// failure on this path -- never a guess.
    fn resolve_login1_session(
        manager: &zbus::blocking::Proxy<'_>,
    ) -> Result<zbus::zvariant::OwnedObjectPath> {
        if let Ok(path) = manager
            .call::<_, _, zbus::zvariant::OwnedObjectPath>("GetSessionByPID", &(std::process::id(),))
        {
            return Ok(path);
        }

        let uid = Self::current_uid()?;

        // `ListSessions` row shape: (session_id, uid, user_name, seat_id,
        // object_path). A seat-less row is a background/manager session, not
        // a graphical one -- confirmed live on this machine (session "3" had
        // seat_id "", session "2" had "seat0" and is the real KDE session).
        type SessionRow = (String, u32, String, String, zbus::zvariant::OwnedObjectPath);
        let sessions: Vec<SessionRow> = manager
            .call("ListSessions", &())
            .context("listing login1 sessions")?;

        let mut candidates: Vec<SessionRow> = sessions
            .into_iter()
            .filter(|(_, row_uid, _, seat_id, _)| *row_uid == uid && !seat_id.is_empty())
            .collect();

        if candidates.is_empty() {
            anyhow::bail!("no seated login1 session found for uid {uid}");
        }

        // Deterministic tie-break when this uid owns more than one seated
        // session (e.g. a physical seat plus a remote/VNC seat): sort by
        // session id first so iteration order is never "whatever
        // ListSessions happened to return" (that order is not documented as
        // stable), then prefer whichever session login1 itself marks
        // `Active` -- the one actually receiving input on its seat, i.e.
        // the session a human is really looking at right now, which is
        // exactly the one whose lock state this probe cares about. If none
        // reports `Active` (this process's uid has seated sessions, but none
        // is foregrounded -- plausible right after a fast user switch),
        // fall back to the lowest session id for full determinism rather
        // than picking arbitrarily.
        candidates.sort_by(|a, b| a.0.cmp(&b.0));
        for (_, _, _, _, path) in &candidates {
            let session = zbus::blocking::Proxy::new(
                manager.connection(),
                "org.freedesktop.login1",
                path,
                "org.freedesktop.login1.Session",
            );
            if let Ok(session) = session {
                if session.get_property::<bool>("Active").unwrap_or(false) {
                    return Ok(path.clone());
                }
            }
        }

        Ok(candidates[0].4.clone())
    }

    /// This process's own real uid, read from `/proc/self/status` rather
    /// than via a new dependency (no `libc` crate is in this project's
    /// dependency tree to call `getuid()` through, and adding one is out of
    /// scope for this fix) or an environment variable (`$USER`/`$UID` can be
    /// stale, unset, or spoofed by whatever launched this process -- a
    /// filesystem read of the kernel's own view of this process is the more
    /// robust of the two dependency-free options). The `Uid:` line reports
    /// four values (real, effective, saved, filesystem); the real uid (the
    /// first) is what `ListSessions`' uid column is compared against.
    fn current_uid() -> Result<u32> {
        let status =
            std::fs::read_to_string("/proc/self/status").context("reading /proc/self/status")?;
        let line = status
            .lines()
            .find(|l| l.starts_with("Uid:"))
            .context("no Uid: line in /proc/self/status")?;
        line.split_whitespace()
            .nth(1)
            .context("malformed Uid: line in /proc/self/status")?
            .parse::<u32>()
            .context("non-numeric uid in /proc/self/status")
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

        // See `login1_locked_hint` for what is and is not verified about
        // this signal. A read failure degrades to `false` -- identical to
        // what this probe reported before `LockedHint` was wired in, in
        // every failure case; it differs only when the read actually
        // succeeds and the session actually is locked.
        let locked = self.login1_locked_hint().unwrap_or(false);

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
