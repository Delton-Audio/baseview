#![expect(clippy::unwrap_used, reason = "To be refactored later")]

use std::{
    collections::HashSet,
    ffi::{c_int, c_void},
    ptr,
    ptr::NonNull,
    sync::{LazyLock, RwLock},
};

use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, POINT, WPARAM},
    System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
    UI::WindowsAndMessaging::{
        CallNextHookEx, SetWindowsHookExW, UnhookWindowsHookEx, HC_ACTION, HHOOK, MSG, PM_REMOVE,
        WH_GETMESSAGE, WM_CHAR, WM_KEYDOWN, WM_KEYUP, WM_SYSCHAR, WM_SYSKEYDOWN, WM_SYSKEYUP,
        WM_USER,
    },
};

use super::*;
use crate::wrappers::win32::window::{wnd_proc, HWnd};

// track all windows opened by this instance of baseview
// we use an RwLock here since the vast majority of uses (event interceptions)
// will only need to read from the HashSet
static HOOK_STATE: LazyLock<RwLock<KeyboardHookState>> = LazyLock::new(RwLock::default);

pub(crate) struct KeyboardHookHandle(HWNDWrapper);

#[derive(Default)]
struct KeyboardHookState {
    hook: Option<HHOOK>,
    open_windows: HashSet<HWNDWrapper>,
}

#[derive(Hash, PartialEq, Eq, Clone, Copy)]
struct HWNDWrapper(HWND);

// SAFETY: it's a pointer behind an RwLock. we'll live
unsafe impl Send for KeyboardHookState {}
unsafe impl Sync for KeyboardHookState {}

// SAFETY: we never access the underlying HWND ourselves, just use it as a HashSet entry
unsafe impl Send for HWNDWrapper {}
unsafe impl Sync for HWNDWrapper {}

impl Drop for KeyboardHookHandle {
    fn drop(&mut self) {
        deinit_keyboard_hook(self.0);
    }
}

// initialize keyboard hook
// some DAWs (particularly Ableton) intercept incoming keyboard messages,
// but we're naughty so we intercept them right back
pub(crate) fn init_keyboard_hook(hwnd: HWND) -> KeyboardHookHandle {
    let state = &mut *HOOK_STATE.write().unwrap();

    // register hwnd to global window set
    state.open_windows.insert(HWNDWrapper(hwnd));

    if state.hook.is_some() {
        // keyboard hook already exists, just return handle
        KeyboardHookHandle(HWNDWrapper(hwnd))
    } else {
        // keyboard hook doesn't exist (no windows open before this), create it
        let new_hook = unsafe {
            SetWindowsHookExW(
                WH_GETMESSAGE,
                Some(keyboard_hook_callback),
                GetModuleHandleW(ptr::null()),
                GetCurrentThreadId(),
            )
        };

        state.hook = Some(new_hook);

        KeyboardHookHandle(HWNDWrapper(hwnd))
    }
}

fn deinit_keyboard_hook(hwnd: HWNDWrapper) {
    let state = &mut *HOOK_STATE.write().unwrap();

    state.open_windows.remove(&hwnd);

    if state.open_windows.is_empty() {
        if let Some(hhook) = state.hook {
            unsafe {
                UnhookWindowsHookEx(hhook);
            }

            state.hook = None;
        }
    }
}

unsafe extern "system" fn keyboard_hook_callback(
    n_code: c_int, wparam: WPARAM, lparam: LPARAM,
) -> isize {
    let msg = lparam as *mut MSG;

    if n_code == HC_ACTION as i32 && wparam == PM_REMOVE as usize && offer_message_to_baseview(msg)
    {
        *msg = MSG {
            hwnd: ptr::null_mut(),
            message: WM_USER,
            wParam: 0,
            lParam: 0,
            time: 0,
            pt: POINT { x: 0, y: 0 },
        };

        0
    } else {
        CallNextHookEx(ptr::null_mut(), n_code, wparam, lparam)
    }
}

// check if `msg` is a keyboard message addressed to a window
// in KeyboardHookState::open_windows, and intercept it if so
unsafe fn offer_message_to_baseview(msg: *mut MSG) -> bool {
    let msg = &*msg;

    // if this isn't a keyboard message, ignore it
    match msg.message {
        WM_KEYDOWN | WM_SYSKEYDOWN | WM_KEYUP | WM_SYSKEYUP | WM_CHAR | WM_SYSCHAR => {}

        _ => return false,
    }

    let state = HOOK_STATE.read().unwrap();

    // Prefer the focused window when it is one of ours. For a parented plugin editor the message
    // is usually addressed to the host's window while our child window holds focus and is the real
    // target, so routing purely by `msg.hwnd` would drop the keyboard input on the floor.
    let focused = HWnd::get_focused_window();
    if state.open_windows.contains(&HWNDWrapper(focused)) && has_window_data(focused) {
        drop(state);
        let _ = wnd_proc::<BaseviewWindow>(focused, msg.message, msg.wParam, msg.lParam);

        return true;
    }

    // Otherwise fall back to the window the message was actually addressed to.
    if state.open_windows.contains(&HWNDWrapper(msg.hwnd)) && has_window_data(msg.hwnd) {
        drop(state);
        let _ = wnd_proc::<BaseviewWindow>(msg.hwnd, msg.message, msg.wParam, msg.lParam);

        return true;
    }

    false
}

/// Whether a window still has its state attached.
///
/// A window can linger in `open_windows` briefly after its state has been torn down. Dispatching
/// into `wnd_proc` during that gap would operate on freed state, so both routes above check this
/// first.
unsafe fn has_window_data(hwnd: HWND) -> bool {
    let Some(hwnd) = NonNull::new(hwnd) else { return false };

    // SAFETY: the handle is non-null, and is only ever one this hook recorded when the window was
    // created.
    unsafe { HWnd::from_raw(hwnd).get_userdata_ptr::<c_void>().is_some() }
}
