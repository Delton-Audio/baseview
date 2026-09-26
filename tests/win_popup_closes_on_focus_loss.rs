//! A popup must dismiss itself when focus moves elsewhere, because that is the only way a menu
//! opened from a plugin editor goes away.
#![cfg(target_os = "windows")]

use baseview::dpi::{LogicalSize, PhysicalPosition};
use baseview::{
    Event, EventStatus, HandlerError, Window, WindowContext, WindowHandler, WindowSettings,
    WindowSize,
};
use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const SETTLE_FRAMES: usize = 20;
const FOCUS_AFTER: usize = 5;
const DEADLINE_FRAMES: usize = 60;

struct Owner {
    cx: WindowContext,
    frame: Cell<usize>,
    popup: RefCell<Option<Window>>,
    opened_at: Cell<Option<usize>>,
    focus_moved_at: Cell<Option<usize>>,
    popup_was_open: Arc<AtomicBool>,
    popup_closed_after_focus_moved: Arc<AtomicBool>,
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

impl WindowHandler for Owner {
    fn on_frame(&self) -> Result<(), HandlerError> {
        let frame = self.frame.get().saturating_add(1);
        self.frame.set(frame);

        if frame == SETTLE_FRAMES {
            let settings =
                WindowSettings::new().with_title("popup").with_size(LogicalSize::new(100.0, 80.0));
            let window =
                match Window::create_popup(PhysicalPosition::new(100, 100), settings, |_cx| {
                    Ok(Noop)
                }) {
                    Ok(window) => window,
                    Err(e) => panic!("could not create the popup window: {e}"),
                };
            self.popup_was_open.store(window.is_open(), Ordering::Relaxed);
            *self.popup.borrow_mut() = Some(window);
            self.opened_at.set(Some(frame));
        }

        if let Some(opened_at) = self.opened_at.get() {
            if frame == opened_at.saturating_add(FOCUS_AFTER) {
                let _ = self.cx.focus();
                self.focus_moved_at.set(Some(frame));
            }
        }

        if let Some(focus_moved_at) = self.focus_moved_at.get() {
            let popup_closed = self.popup.borrow().as_ref().is_some_and(|popup| !popup.is_open());
            if popup_closed {
                self.popup_closed_after_focus_moved.store(true, Ordering::Relaxed);
                self.cx.request_close();
            } else if frame > focus_moved_at.saturating_add(DEADLINE_FRAMES) {
                self.cx.request_close();
            }
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

#[test]
fn a_popup_closes_itself_when_focus_moves_to_another_window() {
    let popup_was_open = Arc::new(AtomicBool::new(false));
    let popup_closed_after_focus_moved = Arc::new(AtomicBool::new(false));
    let handler_popup_was_open = Arc::clone(&popup_was_open);
    let handler_popup_closed_after_focus_moved = Arc::clone(&popup_closed_after_focus_moved);

    let settings =
        WindowSettings::new().with_title("popup owner").with_size(LogicalSize::new(320.0, 240.0));
    let window = match Window::create(settings, move |cx: WindowContext| {
        Ok(Owner {
            cx,
            frame: Cell::new(0),
            popup: RefCell::new(None),
            opened_at: Cell::new(None),
            focus_moved_at: Cell::new(None),
            popup_was_open: handler_popup_was_open,
            popup_closed_after_focus_moved: handler_popup_closed_after_focus_moved,
        })
    }) {
        Ok(window) => window,
        Err(e) => panic!("could not create the owner window: {e}"),
    };
    if let Err(e) = window.run_until_closed() {
        panic!("the owner window's message loop failed: {e}");
    }

    assert!(popup_was_open.load(Ordering::Relaxed), "create_popup must return an open window");
    assert!(
        popup_closed_after_focus_moved.load(Ordering::Relaxed),
        "focus moved to the owner and the popup stayed open -- a menu opened from an editor would never go away"
    );
}
