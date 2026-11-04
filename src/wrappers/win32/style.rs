use crate::WindowSettings;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

#[derive(Copy, Clone)]
pub struct WindowStyle {
    pub style: WINDOW_STYLE,
    pub style_ex: WINDOW_EX_STYLE,
}

impl WindowStyle {
    pub fn from_settings(settings: &WindowSettings) -> Self {
        if settings.parent.is_some() || settings.wait_for_parent {
            return Self { style: WS_CHILD, style_ex: 0 };
        }

        let mut style = Self {
            style: WS_POPUPWINDOW | WS_CAPTION | WS_MINIMIZEBOX | WS_CLIPSIBLINGS,
            style_ex: 0,
        };

        if settings.resizable {
            style.style |= WS_SIZEBOX | WS_MAXIMIZEBOX;
        }

        style
    }

    /// A borderless top-level window for transient menus and pickers.
    ///
    /// Unlike the top-level style from [`from_settings`](Self::from_settings) it has no caption,
    /// border or sizing frame, so its window rect equals its client rect and it needs no
    /// non-client area adjustment. The extended style keeps it off the taskbar and above the
    /// window it was opened from.
    pub const fn popup() -> Self {
        Self {
            style: WS_POPUP | WS_VISIBLE | WS_CLIPSIBLINGS,
            style_ex: WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
        }
    }

    /// A borderless popup that is never activated: [`popup`](Self::popup) plus
    /// `WS_EX_NOACTIVATE`, and without `WS_VISIBLE`, so it is shown explicitly without activation.
    pub const fn owned_popup() -> Self {
        Self {
            style: WS_POPUP | WS_CLIPSIBLINGS,
            style_ex: WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
        }
    }

    /// A borderless tooltip window: never activated and click-through. Like
    /// [`owned_popup`](Self::owned_popup) it has no `WS_VISIBLE`, so it is shown explicitly without
    /// activation, and it adds `WS_EX_TRANSPARENT` so the pointer passes to the window beneath.
    ///
    /// It deliberately has no `WS_EX_LAYERED`: an opaque window presented through softbuffer would
    /// be invisible as a layered window that never gets its layered attributes set.
    pub const fn tooltip() -> Self {
        Self {
            style: WS_POPUP | WS_CLIPSIBLINGS,
            style_ex: WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT,
        }
    }
}
