use std::ptr::null_mut;
use windows_core::{Error, Result};
use windows_sys::Win32::Graphics::Gdi::{GetDC, GetDeviceCaps, ReleaseDC, VREFRESH};
use windows_sys::Win32::Media::{timeBeginPeriod, timeEndPeriod};

/// Raises the process-wide multimedia timer resolution for as long as this guard is alive.
///
/// `SetTimer`'s granularity is the system tick, which defaults to ~15.6 ms and therefore pins a
/// `WM_TIMER`-driven frame loop to ~64 fps. Raising the resolution lets the tick land close to the
/// requested interval instead.
///
/// Note that `SetTimer` still clamps the interval itself to `USER_TIMER_MINIMUM` (10 ms), so this
/// lifts the ceiling to ~100 fps rather than removing it. `WindowContext::request_frame` does not
/// raise that ceiling either: it buys a prompt frame after input, not a faster cadence.
pub struct TimerResolution {
    period_ms: u32,
}

impl TimerResolution {
    /// Requests `period_ms` timer resolution, balanced by `timeEndPeriod` when the guard drops.
    pub fn begin(period_ms: u32) -> Result<Self> {
        // SAFETY: always safe to call. The matching `timeEndPeriod` runs in `Drop`, which is
        // required by the Win32 contract and is why this is exposed as a guard rather than a
        // bare function.
        match unsafe { timeBeginPeriod(period_ms) } {
            0 => Ok(Self { period_ms }),
            _ => Err(Error::from_thread()),
        }
    }
}

impl Drop for TimerResolution {
    fn drop(&mut self) {
        // SAFETY: balances the `timeBeginPeriod` in `begin`, which must have succeeded for this
        // value to exist. The period matches, as Win32 requires.
        unsafe { timeEndPeriod(self.period_ms) };
    }
}

/// The frame interval in milliseconds matching the primary monitor's refresh rate.
///
/// Falls back to ~60 fps when the driver reports no usable rate (Win32 uses 0 and 1 to mean
/// "hardware default", i.e. unknown).
pub fn monitor_frame_interval_ms() -> u32 {
    /// ~60 fps, used when the refresh rate cannot be determined.
    const FALLBACK_MS: u32 = 16;

    // SAFETY: a NULL window handle requests a device context for the entire screen, which is a
    // documented and valid argument.
    let dc = unsafe { GetDC(null_mut()) };
    if dc.is_null() {
        return FALLBACK_MS;
    }

    // SAFETY: `dc` is a valid screen DC, null-checked directly above.
    let hz = unsafe { GetDeviceCaps(dc, VREFRESH as i32) };

    // SAFETY: releases the DC acquired above; the NULL window handle matches the `GetDC` call.
    unsafe { ReleaseDC(null_mut(), dc) };

    if hz > 1 {
        // Floor so we request at or slightly below the refresh period rather than above it.
        ((1000.0 / f64::from(hz)).floor() as u32).max(1)
    } else {
        FALLBACK_MS
    }
}
