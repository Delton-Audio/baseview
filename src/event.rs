use crate::dpi::PhysicalPosition;
use keyboard_types::{KeyboardEvent, Modifiers};
use std::path::PathBuf;

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
#[non_exhaustive]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    Back,
    Forward,
    Other(u8),
}

/// A scroll movement.
#[derive(Debug, Clone, Copy, PartialEq)]
#[expect(clippy::exhaustive_enums, reason = "We don't expect new scroll types anytime soon")]
pub enum ScrollDelta {
    /// A line-based scroll movement.
    Lines {
        /// The number of horizontal lines scrolled.
        x: f32,

        /// The number of vertical lines scrolled.
        y: f32,
    },
    /// A pixel-based scroll movement.
    Pixels {
        /// The number of horizontal pixels scrolled.
        x: f32,
        /// The number of vertical pixels scrolled.
        y: f32,
    },
}

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum MouseEvent {
    /// The mouse cursor was moved.
    CursorMoved {
        /// The logical coordinates of the mouse position.
        position: PhysicalPosition<f64>,
        /// The modifiers that were held down just before the event.
        modifiers: Modifiers,
    },

    /// A mouse button was pressed.
    ButtonPressed {
        /// The button that was pressed.
        button: MouseButton,
        /// The modifiers that were held down just before the event.
        modifiers: Modifiers,
    },

    /// A mouse button was released.
    ButtonReleased {
        /// The button that was released.
        button: MouseButton,
        /// The modifiers that were held down just before the event.
        modifiers: Modifiers,
    },

    /// The mouse wheel was scrolled.
    WheelScrolled {
        /// How much was scrolled, in factional lines.
        delta: ScrollDelta,
        /// The modifiers that were held down just before the event.
        modifiers: Modifiers,
    },

    /// The mouse cursor entered the window.
    ///
    /// May not be available on all platforms.
    CursorEntered,

    /// The mouse cursor left the window.
    ///
    /// May not be available on all platforms.
    CursorLeft,

    DragEntered {
        /// The logical coordinates of the mouse position.
        position: PhysicalPosition<f64>,
        /// The modifiers that were held down just before the event.
        modifiers: Modifiers,
        /// Data being dragged.
        data: DropData,
    },

    DragMoved {
        /// The logical coordinates of the mouse position.
        position: PhysicalPosition<f64>,
        /// The modifiers that were held down just before the event.
        modifiers: Modifiers,
        /// Data being dragged.
        data: DropData,
    },

    DragLeft,

    DragDropped {
        /// The logical coordinates of the mouse position.
        position: PhysicalPosition<f64>,
        /// The modifiers that were held down just before the event.
        modifiers: Modifiers,
        /// Data being dragged.
        data: DropData,
    },
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum WindowEvent {
    Focused,
    Unfocused,
    WillClose,
    /// The window system discarded some of this window's pixels and wants them back -- the window
    /// was uncovered, revealed by another window moving away, or otherwise invalidated from
    /// outside.
    ///
    /// A handler that repaints unconditionally can ignore this. One that skips painting when its
    /// own state has not changed **must** treat it as a reason to repaint anyway: the content is
    /// unchanged, but the screen no longer shows it, and no other event reports that. Focus does
    /// not substitute -- a parented (plugin editor) window is a child window and does not receive
    /// focus when its host is activated.
    ///
    /// Only emitted for invalidation the window system originated. A frame the handler asked for
    /// through `WindowContext::request_frame` travels on its own platform message and never
    /// produces this event, so treating every `Expose` as a reason to repaint cannot drive a loop.
    ///
    /// Currently only emitted on Windows.
    Expose,
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Event {
    Mouse(MouseEvent),
    Keyboard(KeyboardEvent),
    Window(WindowEvent),
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DropEffect {
    Copy,
    Move,
    Link,
    Scroll,
}

#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub enum DropData {
    None,
    Files(Vec<PathBuf>),
}

/// Return value for [WindowHandler::on_event](`crate::WindowHandler::on_event()`),
/// indicating whether the event was handled by your window or should be passed
/// back to the platform.
///
/// For most event types, this value won't have any effect. This is the case
/// when there is no clear meaning of passing back the event to the platform,
/// or it isn't obviously useful. Currently, only [`Event::Keyboard`] variants
/// are supported.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum EventStatus {
    /// Event was handled by your window and will not be sent back to the
    /// platform for further processing.
    Captured,
    /// Event was **not** handled by your window, so pass it back to the
    /// platform. For parented windows, this usually means that the parent
    /// window will receive the event. This is useful for cases such as using
    /// DAW functionality for playing piano keys with the keyboard while a
    /// plugin window is in focus.
    Ignored,
    /// We are prepared to handle the data in the drag and dropping will
    /// result in [DropEffect].
    AcceptDrop(DropEffect),
}
