//! The window system's damage report must survive a frame the handler asked for.
//!
//! This is a regression test for a real, shipped bug. `request_frame` used to be `InvalidateRect`
//! -- the same signal Windows uses to say "your pixels are gone" -- so a requested frame and a
//! genuine expose coalesced onto one indistinguishable `WM_PAINT`. baseview broke the tie with a
//! flag and assumed a requested frame is always one the handler meant to paint. It is not: a
//! handler asks for frames while an animation or a self-driven component is running, which says
//! nothing about whether *this* frame will paint. Whenever such a request was outstanding, the
//! `WM_PAINT` carrying real damage was read as ours, `Expose` was suppressed, and the damage was
//! validated away with nothing repainting it -- a plugin editor dragged across the screen kept
//! whatever pixels the drag left behind.
//!
//! The anchor is the operating system itself: nothing in this test decides what an expose is.
//! We invalidate through the Win32 API directly, exactly as the window system does, and assert the
//! handler is told. Running it needs a real window and a real message queue, so it is Windows-only
//! and will not pass on a headless session.
#![cfg(target_os = "windows")]

use baseview::dpi::LogicalSize;
use baseview::{
    Event, EventStatus, HandlerError, Window, WindowContext, WindowEvent, WindowHandler,
    WindowSettings, WindowSize,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::Graphics::Gdi::InvalidateRect;

/// Frames to let pass before arming, so the paints Windows issues while the window is first shown
/// are drained and cannot be mistaken for the one we are testing.
const SETTLE_FRAMES: usize = 20;
/// Frames to keep asking for a frame before invalidating, so a request is unambiguously in flight.
const ARMED_FRAMES: usize = 5;
/// Frames to wait for the expose after invalidating, before giving up and failing.
const DEADLINE_FRAMES: usize = 40;

struct Probe {
    cx: WindowContext,
    hwnd: HWND,
    frame: Cell<usize>,
    invalidated_at: Cell<Option<usize>>,
    /// Exposes seen strictly after we invalidated. Shared out, because the handler is consumed by
    /// the window and only this survives it.
    exposes_after_invalidate: Arc<AtomicUsize>,
}

impl WindowHandler for Probe {
    fn on_frame(&self) -> Result<(), HandlerError> {
        let frame = self.frame.get().saturating_add(1);
        self.frame.set(frame);

        if frame <= SETTLE_FRAMES {
            return Ok(());
        }

        // From here on the handler behaves like an animating scene: it wants another frame every
        // frame. This is the state that used to swallow the window system's damage report.
        self.cx.request_frame();

        match self.invalidated_at.get() {
            None => {
                if frame > SETTLE_FRAMES + ARMED_FRAMES {
                    self.invalidated_at.set(Some(frame));
                    // Stand in for the window system discarding our pixels -- a drag, an uncover,
                    // a reveal. A NULL rect is the whole client area and 0 means do not erase,
                    // which is exactly what a move over this window produces.
                    //
                    // SAFETY: the window is alive for the duration of the handler, and a NULL rect
                    // is the documented way to name the entire client area.
                    unsafe {
                        InvalidateRect(self.hwnd, std::ptr::null(), 0);
                    }
                }
            }
            Some(at) => {
                let seen = self.exposes_after_invalidate.load(Ordering::Relaxed) > 0;
                if seen || frame > at.saturating_add(DEADLINE_FRAMES) {
                    self.cx.request_close();
                }
            }
        }
        Ok(())
    }

    fn on_event(&self, event: Event) -> EventStatus {
        if let Event::Window(WindowEvent::Expose) = event {
            if self.invalidated_at.get().is_some() {
                self.exposes_after_invalidate.fetch_add(1, Ordering::Relaxed);
            }
        }
        EventStatus::Ignored
    }

    fn resized(&self, _new_size: WindowSize) -> Result<(), HandlerError> {
        Ok(())
    }
}

#[test]
fn an_os_expose_is_reported_even_while_a_frame_request_is_outstanding() {
    let exposes = Arc::new(AtomicUsize::new(0));
    let handler_exposes = Arc::clone(&exposes);

    let settings =
        WindowSettings::new().with_title("expose probe").with_size(LogicalSize::new(320.0, 240.0));
    let window = match Window::create(settings, move |cx: WindowContext| {
        let hwnd = match cx.window_handle() {
            Ok(handle) => match handle.as_raw() {
                RawWindowHandle::Win32(handle) => handle.hwnd.get() as HWND,
                other => panic!("expected a Win32 handle, got {other:?}"),
            },
            Err(e) => panic!("could not get a Win32 handle: {e}"),
        };

        Ok(Probe {
            cx,
            hwnd,
            frame: Cell::new(0),
            invalidated_at: Cell::new(None),
            exposes_after_invalidate: handler_exposes,
        })
    }) {
        Ok(window) => window,
        Err(e) => panic!("could not create the probe window: {e}"),
    };
    if let Err(e) = window.run_until_closed() {
        panic!("the probe window's message loop failed: {e}");
    }

    assert!(
        exposes.load(Ordering::Relaxed) > 0,
        "the window system invalidated the whole client area and the handler was never told, so \
         nothing would repaint it -- this is the dragged-plugin-window bug"
    );
}
