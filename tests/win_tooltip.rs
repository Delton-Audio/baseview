//! A tooltip is its own OS window: it never takes the foreground and the pointer passes through it.
#![cfg(target_os = "windows")]

use baseview::dpi::{LogicalSize, PhysicalPosition};
use baseview::{
    Event, EventStatus, HandlerError, Window, WindowContext, WindowHandler, WindowSettings,
    WindowSize,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Arc, Mutex};
use windows_sys::Win32::Foundation::{HWND, RECT};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetActiveWindow, GetFocus};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindow, GetWindowLongPtrW, GetWindowRect, IsWindowVisible,
    SendMessageW, GWL_EXSTYLE, GW_OWNER, HTCLIENT, HTTRANSPARENT, MA_NOACTIVATE, WM_LBUTTONDOWN,
    WM_MOUSEACTIVATE, WM_NCHITTEST, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_EX_TRANSPARENT,
};

static SERIAL: Mutex<()> = Mutex::new(());

#[derive(Clone, Copy)]
enum Scenario {
    Tooltip,
    OwnedPopup,
}

#[derive(Default)]
struct Results {
    owner_hwnd: Arc<AtomicIsize>,
    popup_hwnd: Arc<AtomicIsize>,
    owned_hwnd: Arc<AtomicIsize>,
    foreground_before: AtomicIsize,
    foreground_after: AtomicIsize,
    focus_after: AtomicIsize,
    active_after: AtomicIsize,
    popup_visible: AtomicBool,
    ex_style: AtomicIsize,
    nchittest_result: AtomicIsize,
    mouseactivate_result: AtomicIsize,
    gw_owner_is_null: AtomicBool,
    owned_nchittest_result: AtomicIsize,
}

struct Owner {
    cx: WindowContext,
    frame: Cell<usize>,
    scenario: Scenario,
    popup: RefCell<Option<Window>>,
    owned: RefCell<Option<Window>>,
    results: Arc<Results>,
}

struct Noop;

impl WindowHandler for Noop {
    fn on_frame(&self) -> Result<(), HandlerError> {
        Ok(())
    }

    fn resized(&self, _new_size: WindowSize) -> Result<(), HandlerError> {
        Ok(())
    }

    fn on_event(&self, _event: Event) -> EventStatus {
        EventStatus::Ignored
    }
}

fn capture_hwnd(cx: &WindowContext, destination: &AtomicIsize) {
    let handle = match cx.window_handle() {
        Ok(handle) => handle,
        Err(e) => panic!("could not get a window handle: {e}"),
    };
    match handle.as_raw() {
        RawWindowHandle::Win32(handle) => destination.store(handle.hwnd.get(), Ordering::Relaxed),
        other => panic!("expected a Win32 handle, got {other:?}"),
    }
}

fn hwnd(value: &AtomicIsize) -> HWND {
    value.load(Ordering::Relaxed) as HWND
}

fn settings() -> WindowSettings {
    WindowSettings::new().with_title("tooltip probe").with_size(LogicalSize::new(100.0, 80.0))
}

/// The `lParam` of a mouse message aimed at the centre of `window`, in screen coordinates.
fn centre_lparam(window: HWND) -> isize {
    let mut rect = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    // SAFETY: `window` was captured from a live window and `rect` is a valid out pointer.
    let ok = unsafe { GetWindowRect(window, &mut rect) };
    assert_ne!(ok, 0, "GetWindowRect failed");
    let x = rect.left.saturating_add(rect.right.saturating_sub(rect.left).saturating_div(2));
    let y = rect.top.saturating_add(rect.bottom.saturating_sub(rect.top).saturating_div(2));
    ((x as i16 as u16 as isize) & 0xFFFF) | (((y as i16 as u16 as isize) & 0xFFFF) << 16)
}

impl WindowHandler for Owner {
    fn on_frame(&self) -> Result<(), HandlerError> {
        let frame = self.frame.get().saturating_add(1);
        self.frame.set(frame);
        let r = &self.results;

        if frame == 20 {
            // Whatever window is in the foreground now (this test window, or the developer's own app):
            // the tooltip must leave it there. No synthetic input is sent to make this window the
            // foreground; that would press keys on the developer's desktop.
            // SAFETY: GetForegroundWindow takes no arguments.
            let foreground = unsafe { GetForegroundWindow() };
            r.foreground_before.store(foreground as isize, Ordering::Relaxed);

            let handle = Arc::clone(&r.popup_hwnd);
            let created = match self.scenario {
                Scenario::Tooltip => {
                    Window::create_tooltip(PhysicalPosition::new(300, 300), settings(), move |cx| {
                        capture_hwnd(&cx, &handle);
                        Ok(Noop)
                    })
                }
                Scenario::OwnedPopup => {
                    Window::create_popup(PhysicalPosition::new(300, 300), settings(), move |cx| {
                        capture_hwnd(&cx, &handle);
                        Ok(Noop)
                    })
                }
            };
            match created {
                Ok(window) => *self.popup.borrow_mut() = Some(window),
                Err(e) => panic!("could not create the popup: {e}"),
            }
        }

        // Everything is checked in the frame that creates the windows: another application taking
        // the foreground a moment later (a focus-loss dismissal of the root popup, say) is the
        // machine's doing, not the code's.
        if frame == 20 && matches!(self.scenario, Scenario::OwnedPopup) {
            let popup = self.popup.borrow();
            let Some(popup) = popup.as_ref() else { panic!("the root popup is missing") };
            let handle = Arc::clone(&r.owned_hwnd);
            match Window::create_owned_popup(
                popup,
                PhysicalPosition::new(500, 300),
                settings(),
                move |cx| {
                    capture_hwnd(&cx, &handle);
                    Ok(Noop)
                },
            ) {
                Ok(window) => *self.owned.borrow_mut() = Some(window),
                Err(e) => panic!("could not create the owned popup: {e}"),
            }
        }

        if frame == 20 {
            match self.scenario {
                Scenario::Tooltip => {
                    let tooltip = hwnd(&r.popup_hwnd);
                    // SAFETY: the HWNDs were captured from live windows on this thread.
                    unsafe {
                        r.foreground_after.store(GetForegroundWindow() as isize, Ordering::Relaxed);
                        // Thread-level activation and focus work without the foreground, so these
                        // catch an activating show even when Windows refuses this process the
                        // foreground.
                        r.focus_after.store(GetFocus() as isize, Ordering::Relaxed);
                        r.active_after.store(GetActiveWindow() as isize, Ordering::Relaxed);
                        r.popup_visible.store(IsWindowVisible(tooltip) != 0, Ordering::Relaxed);
                        r.ex_style
                            .store(GetWindowLongPtrW(tooltip, GWL_EXSTYLE), Ordering::Relaxed);
                        r.nchittest_result.store(
                            SendMessageW(tooltip, WM_NCHITTEST, 0, centre_lparam(tooltip)),
                            Ordering::Relaxed,
                        );
                        let lparam = ((WM_LBUTTONDOWN << 16) | HTCLIENT) as isize;
                        r.mouseactivate_result.store(
                            SendMessageW(
                                tooltip,
                                WM_MOUSEACTIVATE,
                                hwnd(&r.owner_hwnd) as usize,
                                lparam,
                            ),
                            Ordering::Relaxed,
                        );
                        r.gw_owner_is_null
                            .store(GetWindow(tooltip, GW_OWNER).is_null(), Ordering::Relaxed);
                    }
                }
                Scenario::OwnedPopup => {
                    let owned = hwnd(&r.owned_hwnd);
                    // SAFETY: the HWND was captured from a live window on this thread.
                    unsafe {
                        r.owned_nchittest_result.store(
                            SendMessageW(owned, WM_NCHITTEST, 0, centre_lparam(owned)),
                            Ordering::Relaxed,
                        );
                    }
                }
            }
            self.cx.request_close();
        }

        if frame > 80 {
            self.cx.request_close();
        }
        Ok(())
    }

    fn resized(&self, _new_size: WindowSize) -> Result<(), HandlerError> {
        Ok(())
    }

    fn on_event(&self, _event: Event) -> EventStatus {
        EventStatus::Ignored
    }
}

fn run(scenario: Scenario) -> Arc<Results> {
    let results = Arc::new(Results::default());
    let handler_results = Arc::clone(&results);
    let window = match Window::create(settings(), move |cx| {
        capture_hwnd(&cx, &handler_results.owner_hwnd);
        Ok(Owner {
            cx,
            frame: Cell::new(0),
            scenario,
            popup: RefCell::new(None),
            owned: RefCell::new(None),
            results: handler_results,
        })
    }) {
        Ok(window) => window,
        Err(e) => panic!("could not create the first window: {e}"),
    };
    if let Err(e) = window.run_until_closed() {
        panic!("the message loop failed: {e}");
    }
    results
}

#[test]
fn a_tooltip_never_takes_the_foreground() {
    let _serial = SERIAL.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let r = run(Scenario::Tooltip);
    let tooltip = r.popup_hwnd.load(Ordering::Relaxed);
    assert_eq!(
        r.foreground_after.load(Ordering::Relaxed),
        r.foreground_before.load(Ordering::Relaxed),
        "the tooltip changed the foreground window (tooltip {tooltip:#x})"
    );
    assert_ne!(r.focus_after.load(Ordering::Relaxed), tooltip, "the tooltip took keyboard focus");
    assert_ne!(
        r.active_after.load(Ordering::Relaxed),
        tooltip,
        "the tooltip became the active window"
    );
    assert!(r.popup_visible.load(Ordering::Relaxed), "the tooltip must be visible");
}

#[test]
fn a_tooltip_is_click_through_topmost_and_never_activates() {
    let _serial = SERIAL.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let r = run(Scenario::Tooltip);
    let ex = r.ex_style.load(Ordering::Relaxed);
    assert_ne!(ex & WS_EX_TRANSPARENT as isize, 0, "WS_EX_TRANSPARENT missing");
    assert_ne!(ex & WS_EX_NOACTIVATE as isize, 0, "WS_EX_NOACTIVATE missing");
    assert_ne!(ex & WS_EX_TOPMOST as isize, 0, "WS_EX_TOPMOST missing");
    assert_ne!(ex & WS_EX_TOOLWINDOW as isize, 0, "WS_EX_TOOLWINDOW missing");
    assert_eq!(ex & WS_EX_LAYERED as isize, 0, "WS_EX_LAYERED must not be set");
    assert_eq!(
        r.nchittest_result.load(Ordering::Relaxed),
        HTTRANSPARENT as isize,
        "WM_NCHITTEST must answer HTTRANSPARENT"
    );
    assert_eq!(r.mouseactivate_result.load(Ordering::Relaxed), MA_NOACTIVATE as isize);
}

#[test]
fn a_tooltip_has_no_owner() {
    let _serial = SERIAL.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let r = run(Scenario::Tooltip);
    assert!(r.gw_owner_is_null.load(Ordering::Relaxed), "a tooltip must have no owner");
}

#[test]
fn the_other_popups_keep_their_hit_testing() {
    let _serial = SERIAL.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let r = run(Scenario::OwnedPopup);
    assert_eq!(
        r.owned_nchittest_result.load(Ordering::Relaxed),
        HTCLIENT as isize,
        "an owned popup must still hit-test as its client area"
    );
}
