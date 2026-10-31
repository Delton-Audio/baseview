//! Win32 ownership and focus are the anchors for popup-chain lifetime and activation.
#![cfg(target_os = "windows")]

use baseview::dpi::{LogicalSize, PhysicalPosition};
use baseview::{
    Event, EventStatus, HandlerError, Window, WindowContext, WindowHandler, WindowSettings,
    WindowSize,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetFocus;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetWindow, GetWindowLongPtrW, IsWindow, SendMessageW, GWL_EXSTYLE, GW_OWNER, HTCLIENT,
    MA_NOACTIVATE, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEACTIVATE, WS_EX_NOACTIVATE,
};

static SERIAL: Mutex<()> = Mutex::new(());

#[derive(Clone, Copy)]
enum Scenario {
    KeepFocus,
    CloseChain,
    DropOwned,
    ClosedOwner,
}

#[derive(Default)]
struct Results {
    owner_hwnd: Arc<AtomicIsize>,
    root_hwnd: Arc<AtomicIsize>,
    a_hwnd: Arc<AtomicIsize>,
    b_hwnd: Arc<AtomicIsize>,
    all_open: AtomicBool,
    root_focused: AtomicBool,
    a_owned_by_root: AtomicBool,
    b_owned_by_a: AtomicBool,
    noactivate_style: AtomicBool,
    mouseactivate_result: AtomicIsize,
    focused_after_message: AtomicBool,
    open_after_message: AtomicBool,
    focused_after_click: AtomicBool,
    open_at_40: AtomicBool,
    focused_at_40: AtomicBool,
    a_destroyed: AtomicBool,
    root_closed_at: AtomicUsize,
    a_closed_at: AtomicUsize,
    b_closed_at: AtomicUsize,
    closed_owner_error: AtomicBool,
}

struct Owner {
    cx: WindowContext,
    frame: Cell<usize>,
    scenario: Scenario,
    root: RefCell<Option<Window>>,
    a: RefCell<Option<Window>>,
    b: RefCell<Option<Window>>,
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
    WindowSettings::new().with_title("popup probe").with_size(LogicalSize::new(100.0, 80.0))
}

fn is_open(window: &RefCell<Option<Window>>) -> bool {
    window.borrow().as_ref().is_some_and(Window::is_open)
}

fn owned_popup(
    owner: &RefCell<Option<Window>>, x: i32, handle: Arc<AtomicIsize>,
) -> Option<Window> {
    let owner = owner.borrow();
    let owner = owner.as_ref()?;
    Window::create_owned_popup(owner, PhysicalPosition::new(x, 100), settings(), move |cx| {
        capture_hwnd(&cx, &handle);
        Ok(Noop)
    })
    .ok()
}

fn record_closed(window: &RefCell<Option<Window>>, frame: usize, result: &AtomicUsize) {
    if window.borrow().as_ref().is_some_and(|window| !window.is_open()) {
        let _ = result.compare_exchange(0, frame, Ordering::Relaxed, Ordering::Relaxed);
    }
}

impl WindowHandler for Owner {
    fn on_frame(&self) -> Result<(), HandlerError> {
        let frame = self.frame.get().saturating_add(1);
        self.frame.set(frame);
        let r = &self.results;

        if frame == 20 {
            let handle = Arc::clone(&r.root_hwnd);
            let root =
                match Window::create_popup(PhysicalPosition::new(100, 100), settings(), move |cx| {
                    capture_hwnd(&cx, &handle);
                    Ok(Noop)
                }) {
                    Ok(window) => window,
                    Err(e) => panic!("could not create the root popup: {e}"),
                };
            *self.root.borrow_mut() = Some(root);
        }

        if frame == 25 {
            if matches!(self.scenario, Scenario::ClosedOwner) {
                let _ = self.cx.focus();
            } else {
                *self.a.borrow_mut() = owned_popup(&self.root, 220, Arc::clone(&r.a_hwnd));
            }
        }
        if frame == 27 && matches!(self.scenario, Scenario::KeepFocus | Scenario::CloseChain) {
            *self.b.borrow_mut() = owned_popup(&self.a, 340, Arc::clone(&r.b_hwnd));
        }
        if frame == 30 && matches!(self.scenario, Scenario::DropOwned) {
            r.all_open.store(is_open(&self.root) && is_open(&self.a), Ordering::Relaxed);
            drop(self.a.borrow_mut().take());
        }

        if frame == 35 {
            match self.scenario {
                Scenario::KeepFocus => {
                    r.all_open.store(
                        is_open(&self.root) && is_open(&self.a) && is_open(&self.b),
                        Ordering::Relaxed,
                    );
                    // SAFETY: these HWNDs were captured from the live windows on this thread.
                    unsafe {
                        r.root_focused.store(GetFocus() == hwnd(&r.root_hwnd), Ordering::Relaxed);
                        r.a_owned_by_root.store(
                            GetWindow(hwnd(&r.a_hwnd), GW_OWNER) == hwnd(&r.root_hwnd),
                            Ordering::Relaxed,
                        );
                        r.b_owned_by_a.store(
                            GetWindow(hwnd(&r.b_hwnd), GW_OWNER) == hwnd(&r.a_hwnd),
                            Ordering::Relaxed,
                        );
                        r.noactivate_style.store(
                            GetWindowLongPtrW(hwnd(&r.a_hwnd), GWL_EXSTYLE)
                                & WS_EX_NOACTIVATE as isize
                                != 0,
                            Ordering::Relaxed,
                        );
                        let lparam = ((WM_LBUTTONDOWN << 16) | HTCLIENT) as isize;
                        r.mouseactivate_result.store(
                            SendMessageW(
                                hwnd(&r.a_hwnd),
                                WM_MOUSEACTIVATE,
                                hwnd(&r.root_hwnd) as usize,
                                lparam,
                            ),
                            Ordering::Relaxed,
                        );
                        r.focused_after_message
                            .store(GetFocus() == hwnd(&r.root_hwnd), Ordering::Relaxed);
                        // Deliver the click too: WM_MOUSEACTIVATE alone cannot detect a handler
                        // explicitly taking focus while processing WM_LBUTTONDOWN.
                        SendMessageW(hwnd(&r.a_hwnd), WM_LBUTTONDOWN, 0, 0);
                        SendMessageW(hwnd(&r.a_hwnd), WM_LBUTTONUP, 0, 0);
                        r.focused_after_click
                            .store(GetFocus() == hwnd(&r.root_hwnd), Ordering::Relaxed);
                    }
                    r.open_after_message.store(is_open(&self.root), Ordering::Relaxed);
                }
                Scenario::CloseChain => {
                    r.all_open.store(
                        is_open(&self.root) && is_open(&self.a) && is_open(&self.b),
                        Ordering::Relaxed,
                    );
                    let _ = self.cx.focus();
                    // SAFETY: GetFocus reads this thread's focus; the owner handle is captured.
                    r.root_focused
                        .store(unsafe { GetFocus() == hwnd(&r.owner_hwnd) }, Ordering::Relaxed);
                }
                Scenario::DropOwned | Scenario::ClosedOwner => {}
            }
        }

        if frame == 40 && matches!(self.scenario, Scenario::KeepFocus | Scenario::DropOwned) {
            r.open_at_40.store(is_open(&self.root), Ordering::Relaxed);
            // SAFETY: GetFocus reads this thread's focus. IsWindow accepts a destroyed HWND.
            unsafe {
                r.focused_at_40.store(GetFocus() == hwnd(&r.root_hwnd), Ordering::Relaxed);
                r.a_destroyed.store(IsWindow(hwnd(&r.a_hwnd)) == 0, Ordering::Relaxed);
            }
            self.cx.request_close();
        }

        if frame > 35 && matches!(self.scenario, Scenario::CloseChain) {
            record_closed(&self.root, frame, &r.root_closed_at);
            record_closed(&self.a, frame, &r.a_closed_at);
            record_closed(&self.b, frame, &r.b_closed_at);
            if r.root_closed_at.load(Ordering::Relaxed) != 0
                && r.a_closed_at.load(Ordering::Relaxed) != 0
                && r.b_closed_at.load(Ordering::Relaxed) != 0
            {
                self.cx.request_close();
            }
        }

        if frame > 25 && matches!(self.scenario, Scenario::ClosedOwner) {
            let root = self.root.borrow();
            if let Some(root) = root.as_ref().filter(|root| !root.is_open()) {
                let handle = Arc::clone(&r.a_hwnd);
                let result = Window::create_owned_popup(
                    root,
                    PhysicalPosition::new(220, 100),
                    settings(),
                    move |cx| {
                        capture_hwnd(&cx, &handle);
                        Ok(Noop)
                    },
                );
                r.closed_owner_error.store(
                    matches!(result, Err(e) if e.to_string() == "The popup's owner window is not open."),
                    Ordering::Relaxed,
                );
                self.cx.request_close();
            }
        }

        let deadline = match self.scenario {
            Scenario::KeepFocus | Scenario::CloseChain => 95,
            Scenario::DropOwned => 90,
            Scenario::ClosedOwner => 85,
        };
        if frame > deadline {
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
            root: RefCell::new(None),
            a: RefCell::new(None),
            b: RefCell::new(None),
            results: handler_results,
        })
    }) {
        Ok(window) => window,
        Err(e) => panic!("could not create the owner window: {e}"),
    };
    if let Err(e) = window.run_until_closed() {
        panic!("the owner's message loop failed: {e}");
    }
    results
}

#[test]
fn an_owned_popup_takes_no_focus_and_keeps_its_owner_open() {
    let _serial = SERIAL.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let r = run(Scenario::KeepFocus);
    assert!(r.root_focused.load(Ordering::Relaxed), "focus must stay on R");
    assert!(r.all_open.load(Ordering::Relaxed), "R, A and B must stay open");
    assert!(r.a_owned_by_root.load(Ordering::Relaxed), "GW_OWNER(A) must be R");
    assert!(r.b_owned_by_a.load(Ordering::Relaxed), "GW_OWNER(B) must be A");
    assert!(r.noactivate_style.load(Ordering::Relaxed), "A must have WS_EX_NOACTIVATE");
    assert_eq!(r.mouseactivate_result.load(Ordering::Relaxed), MA_NOACTIVATE as isize);
    assert!(r.focused_after_message.load(Ordering::Relaxed), "WM_MOUSEACTIVATE moved focus");
    assert!(r.open_after_message.load(Ordering::Relaxed), "WM_MOUSEACTIVATE closed R");
    assert!(r.focused_after_click.load(Ordering::Relaxed), "WM_LBUTTONDOWN moved focus");
    assert!(r.open_at_40.load(Ordering::Relaxed), "R must still be open at frame 40");
    assert!(r.focused_at_40.load(Ordering::Relaxed), "R must still have focus at frame 40");
}

#[test]
fn focus_leaving_the_chain_closes_every_popup_in_it() {
    let _serial = SERIAL.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let r = run(Scenario::CloseChain);
    assert!(r.all_open.load(Ordering::Relaxed), "the full chain must exist before moving focus");
    assert!(r.root_focused.load(Ordering::Relaxed), "focus must move to the owner window");
    assert_ne!(r.root_closed_at.load(Ordering::Relaxed), 0, "R never closed");
    assert_ne!(r.a_closed_at.load(Ordering::Relaxed), 0, "A never closed");
    assert_ne!(r.b_closed_at.load(Ordering::Relaxed), 0, "B never closed");
}

#[test]
fn dropping_an_owned_popup_leaves_its_owner_open_and_focused() {
    let _serial = SERIAL.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let r = run(Scenario::DropOwned);
    assert!(r.all_open.load(Ordering::Relaxed), "R and A must exist before dropping A");
    assert!(r.open_at_40.load(Ordering::Relaxed), "dropping A closed R");
    assert!(r.focused_at_40.load(Ordering::Relaxed), "dropping A moved focus from R");
    assert!(r.a_destroyed.load(Ordering::Relaxed), "dropping A must destroy its HWND");
}

#[test]
fn an_owned_popup_needs_an_open_owner() {
    let _serial = SERIAL.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let r = run(Scenario::ClosedOwner);
    assert!(r.closed_owner_error.load(Ordering::Relaxed), "a closed owner must return OwnerClosed");
}
