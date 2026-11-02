//! What Windows itself reports about the cursor a custom cursor makes, and that dropping one frees it.
#![cfg(target_os = "windows")]

use baseview::dpi::LogicalSize;
use baseview::{
    CursorImage, CursorImages, Event, EventStatus, HandlerError, MouseCursor, Window,
    WindowContext, WindowHandler, WindowSettings, WindowSize,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::cell::Cell;
use std::mem::size_of;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::Graphics::Gdi::{
    DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP, BITMAPINFO, BI_RGB,
    DIB_RGB_COLORS,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetGuiResources, GR_USEROBJECTS};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetCursor, GetIconInfo, LoadCursorW, SendMessageW, HCURSOR, HTCLIENT, ICONINFO, IDC_CROSS,
    WM_MOUSEMOVE, WM_SETCURSOR,
};

static SERIAL: Mutex<()> = Mutex::new(());

#[derive(Clone, Copy)]
enum Scenario {
    WhatWindowsShows,
    DroppingFrees,
}

#[derive(Default)]
struct Results {
    ran: AtomicBool,
    failures: Mutex<Vec<String>>,
}

impl Results {
    fn fail(&self, message: String) {
        self.failures.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).push(message);
    }

    fn check(&self, condition: bool, message: &str) {
        if !condition {
            self.fail(message.to_owned());
        }
    }
}

struct Probe {
    cx: WindowContext,
    hwnd: HWND,
    frame: Cell<usize>,
    scenario: Scenario,
    results: Arc<Results>,
}

/// A 32 pixel image at scale 1 and a 64 pixel one at scale 2, with hotspot (3, 5). Every pixel is half transparent red,
/// except the first, which is opaque green.
fn images() -> CursorImages {
    let make = |side: u32, scale: f64| {
        let mut rgba = Vec::new();
        for index in 0..side.saturating_mul(side) {
            if index == 0 {
                rgba.extend_from_slice(&[0, 255, 0, 255]);
            } else {
                rgba.extend_from_slice(&[255, 0, 0, 128]);
            }
        }
        match CursorImage::new(side, side, scale, 3.0, 5.0, rgba) {
            Some(image) => image,
            None => panic!("a valid image was rejected"),
        }
    };

    match CursorImages::new(vec![make(32, 1.0), make(64, 2.0)]) {
        Some(images) => images,
        None => panic!("valid images were rejected"),
    }
}

fn set_cursor_message(hwnd: HWND) {
    let lparam = ((WM_MOUSEMOVE << 16) | HTCLIENT) as isize;
    // SAFETY: `hwnd` is the live window this handler runs on, on its own thread.
    unsafe { SendMessageW(hwnd, WM_SETCURSOR, hwnd as usize, lparam) };
}

/// The colour bitmap's size, the hotspot, and the first two pixels of the cursor Windows currently shows.
struct Shown {
    is_icon: bool,
    width: i32,
    height: i32,
    hotspot: (u32, u32),
    pixels: Vec<u8>,
}

fn shown_cursor(cursor: HCURSOR) -> Option<Shown> {
    let mut info = ICONINFO::default();
    // SAFETY: `info` is a valid out-pointer.
    if unsafe { GetIconInfo(cursor, &mut info) } == 0 {
        return None;
    }

    let mut bitmap = BITMAP::default();
    // SAFETY: `bitmap` is a BITMAP and the size passed is its size.
    let got =
        unsafe { GetObjectW(info.hbmColor, size_of::<BITMAP>() as i32, (&raw mut bitmap).cast()) };

    let mut pixels = Vec::new();
    if got != 0 && bitmap.bmWidth > 0 && bitmap.bmHeight > 0 {
        let mut header = BITMAPINFO::default();
        header.bmiHeader.biSize =
            size_of::<windows_sys::Win32::Graphics::Gdi::BITMAPINFOHEADER>() as u32;
        header.bmiHeader.biWidth = bitmap.bmWidth;
        // A negative height asks for top-down rows.
        header.bmiHeader.biHeight = bitmap.bmHeight.wrapping_neg();
        header.bmiHeader.biPlanes = 1;
        header.bmiHeader.biBitCount = 32;
        header.bmiHeader.biCompression = BI_RGB;

        pixels = vec![
            0u8;
            (bitmap.bmWidth as usize)
                .saturating_mul(bitmap.bmHeight as usize)
                .saturating_mul(4)
        ];
        // SAFETY: `pixels` holds the whole width x height x 4 bytes the header asks for, and the colour bitmap is
        // not selected into any device context.
        let copied = unsafe {
            let hdc = GetDC(null_mut());
            let copied = GetDIBits(
                hdc,
                info.hbmColor,
                0,
                bitmap.bmHeight as u32,
                pixels.as_mut_ptr().cast(),
                &mut header,
                DIB_RGB_COLORS,
            );
            ReleaseDC(null_mut(), hdc);
            copied
        };
        if copied == 0 {
            pixels.clear();
        }
    }

    // SAFETY: GetIconInfo handed over these two bitmaps, which this function now owns.
    unsafe {
        DeleteObject(info.hbmColor);
        DeleteObject(info.hbmMask);
    }

    Some(Shown {
        is_icon: info.fIcon != 0,
        width: bitmap.bmWidth,
        height: bitmap.bmHeight,
        hotspot: (info.xHotspot, info.yHotspot),
        pixels,
    })
}

fn user_objects() -> u32 {
    // SAFETY: the pseudo handle of the current process is always valid.
    unsafe { GetGuiResources(GetCurrentProcess(), GR_USEROBJECTS) }
}

impl Probe {
    fn check_custom_cursor_is_shown(&self, what: &str, expected: &CursorImage) {
        let r = &self.results;
        // SAFETY: GetCursor only reads this thread's current cursor.
        let Some(shown) = shown_cursor(unsafe { GetCursor() }) else {
            r.fail(format!("{what}: GetIconInfo failed on the current cursor"));
            return;
        };

        r.check(!shown.is_icon, &format!("{what}: fIcon must be FALSE for a cursor"));
        r.check(
            shown.width == expected.width() as i32 && shown.height == expected.height() as i32,
            &format!(
                "{what}: expected a {}x{} bitmap, Windows has {}x{}",
                expected.width(),
                expected.height(),
                shown.width,
                shown.height
            ),
        );
        r.check(
            shown.hotspot == expected.physical_hotspot(),
            &format!(
                "{what}: expected hotspot {:?}, Windows has {:?}",
                expected.physical_hotspot(),
                shown.hotspot
            ),
        );
    }

    fn what_windows_shows(&self) {
        let r = &self.results;
        let images = images();
        let expected = images.for_scale(self.cx.scale_factor());

        let cursor = match self.cx.create_custom_cursor(&images) {
            Ok(cursor) => cursor,
            Err(e) => {
                r.fail(format!("create_custom_cursor failed: {e}"));
                return;
            }
        };
        if let Err(e) = self.cx.set_custom_cursor(&cursor) {
            r.fail(format!("set_custom_cursor failed: {e}"));
            return;
        }
        set_cursor_message(self.hwnd);

        self.check_custom_cursor_is_shown("after WM_SETCURSOR", expected);

        // SAFETY: GetCursor only reads this thread's current cursor.
        match shown_cursor(unsafe { GetCursor() }) {
            Some(shown) if shown.pixels.len() >= 8 => {
                r.check(
                    shown.pixels.get(0..4) == Some(&[0, 255, 0, 255][..]),
                    &format!(
                        "pixel (0, 0) must be BGRA [0, 255, 0, 255], is {:?}",
                        shown.pixels.get(0..4)
                    ),
                );
                r.check(
                    shown.pixels.get(4..8) == Some(&[0, 0, 255, 128][..]),
                    &format!(
                        "pixel (1, 0) must be BGRA [0, 0, 255, 128], is {:?}",
                        shown.pixels.get(4..8)
                    ),
                );
            }
            _ => r.fail("could not read the cursor's pixels".to_owned()),
        }

        if let Err(e) = self.cx.set_mouse_cursor(MouseCursor::Crosshair) {
            r.fail(format!("set_mouse_cursor failed: {e}"));
        }
        set_cursor_message(self.hwnd);
        // SAFETY: both calls only read; IDC_CROSS is a shared cursor identifier.
        let (current, cross) = unsafe { (GetCursor(), LoadCursorW(null_mut(), IDC_CROSS)) };
        r.check(current == cross, "after set_mouse_cursor the system crosshair must be current");

        if let Err(e) = self.cx.set_custom_cursor(&cursor) {
            r.fail(format!("second set_custom_cursor failed: {e}"));
        }
        set_cursor_message(self.hwnd);
        self.check_custom_cursor_is_shown("after setting the custom cursor again", expected);
    }

    fn dropping_frees(&self) {
        let r = &self.results;
        let images = images();
        let build = || match self.cx.create_custom_cursor(&images) {
            Ok(cursor) => Some(cursor),
            Err(e) => {
                r.fail(format!("create_custom_cursor failed: {e}"));
                None
            }
        };

        let before = user_objects();

        let held: Vec<_> = (0..50).filter_map(|_| build()).collect();
        let while_held = user_objects();
        r.check(
            while_held >= before.saturating_add(50),
            &format!("holding 50 cursors went from {before} to {while_held} user objects"),
        );
        drop(held);

        for _ in 0..200 {
            drop(build());
        }
        let after = user_objects();
        r.check(
            after <= before.saturating_add(5),
            &format!(
                "creating and dropping 200 cursors went from {before} to {after} user objects"
            ),
        );
    }
}

impl WindowHandler for Probe {
    fn on_frame(&self) -> Result<(), HandlerError> {
        let frame = self.frame.get().saturating_add(1);
        self.frame.set(frame);

        if frame == 3 {
            match self.scenario {
                Scenario::WhatWindowsShows => self.what_windows_shows(),
                Scenario::DroppingFrees => self.dropping_frees(),
            }
            self.results.ran.store(true, Ordering::Relaxed);
            self.cx.request_close();
        }
        if frame > 60 {
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
    let settings = WindowSettings::new()
        .with_title("custom cursor probe")
        .with_size(LogicalSize::new(100.0, 80.0));
    let window = match Window::create(settings, move |cx| {
        let hwnd = match cx.window_handle().map(|handle| handle.as_raw()) {
            Ok(RawWindowHandle::Win32(handle)) => handle.hwnd.get() as HWND,
            Ok(other) => panic!("expected a Win32 handle, got {other:?}"),
            Err(e) => panic!("could not get a window handle: {e}"),
        };
        Ok(Probe { cx, hwnd, frame: Cell::new(0), scenario, results: handler_results })
    }) {
        Ok(window) => window,
        Err(e) => panic!("could not create the window: {e}"),
    };
    if let Err(e) = window.run_until_closed() {
        panic!("the message loop failed: {e}");
    }
    results
}

fn assert_clean(results: &Results) {
    assert!(results.ran.load(Ordering::Relaxed), "the probe never ran");
    let failures = results.failures.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn a_custom_cursor_is_what_windows_shows() {
    let _serial = SERIAL.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    assert_clean(&run(Scenario::WhatWindowsShows));
}

#[test]
fn dropping_custom_cursors_frees_them() {
    let _serial = SERIAL.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    assert_clean(&run(Scenario::DroppingFrees));
}
