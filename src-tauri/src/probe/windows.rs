//! Windows `ActivityProbe`.
//!
//! Built against the Win32 API via the `windows` crate. This implementation
//! is unverified (compiled only, never run on any machine involved in this
//! project). See approximations and comments below for uncertainty about
//! signal reliability.

use super::{ActivityProbe, Sample};
use windows::Win32::System::StationsAndDesktops::{CloseDesktop, OpenInputDesktop, DESKTOP_SWITCHDESKTOP};
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
        // SAFETY: GetTickCount64 takes no arguments and has no preconditions;
        // it simply returns the system tick count as a u64.
        let now = unsafe { GetTickCount64() };
        // `LASTINPUTINFO::dwTime` is a 32-bit tick count that WRAPS roughly
        // every 49.7 days of uptime; `GetTickCount64` does not. Subtracting
        // them as 64-bit values gives a nonsense ~4.29-million-second idle
        // time for the ~49.7 days following each wrap. Truncate the 64-bit
        // clock to 32 bits and subtract in that same wrapping space, which is
        // correct on both sides of a wrap for any real idle duration (the
        // difference only has to fit in 32 bits, i.e. be under 49.7 days).
        let idle_seconds = u64::from((now as u32).wrapping_sub(info.dwTime)) / 1000;

        // SAFETY: no arguments, returns a plain enum.
        let state = unsafe { SHQueryUserNotificationState() }?;
        let presenting = state == QUNS_PRESENTATION_MODE || state == QUNS_RUNNING_D3D_FULL_SCREEN;
        // QUNS_BUSY is an approximation: it means a fullscreen app is running,
        // which may or may not actually prevent display sleep. This is not a real
        // display-state query like the Linux idle protocol or similar assertions
        // elsewhere. It is the best-effort proxy available via the Win32 API.
        let display_held_awake = presenting || state == QUNS_BUSY;

        // OpenInputDesktop fails when the session is locked.
        // SAFETY: handle is closed immediately when the call succeeds.
        let locked = unsafe {
            match OpenInputDesktop(Default::default(), false, DESKTOP_SWITCHDESKTOP) {
                Ok(h) => {
                    let _ = CloseDesktop(h);
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
