use crate::platform::{PlatformError, Result};
use crate::{CursorImages, MouseCursor};
use objc2::__framework_prelude::Retained;
use objc2::runtime::{MessageReceiver, Sel};
use objc2::{msg_send, sel, AnyThread, ClassType, Message};
use objc2_app_kit::{NSBitmapFormat, NSBitmapImageRep, NSCursor, NSDeviceRGBColorSpace, NSImage};
use objc2_foundation::{NSPoint, NSSize};
use std::cell::{Cell, LazyCell, RefCell};
use std::ptr::null_mut;

/// An `NSCursor` built from [`CursorImages`]: one bitmap representation per image, between which AppKit picks by the
/// scale of the screen the cursor is on.
pub struct CustomCursor {
    cursor: Retained<NSCursor>,
}

impl CustomCursor {
    pub fn create(images: &CursorImages) -> Result<Self> {
        let largest = images
            .images()
            .iter()
            .max_by(|a, b| a.scale().total_cmp(&b.scale()))
            .ok_or(PlatformError::CustomCursorFailed)?;

        // The size of the cursor in points is that of its largest image.
        let size = NSSize::new(
            f64::from(largest.width()) / largest.scale(),
            f64::from(largest.height()) / largest.scale(),
        );
        let image = NSImage::initWithSize(NSImage::alloc(), size);

        for source in images.images() {
            let row_len = (source.width() as usize).saturating_mul(4);

            // SAFETY: a null `planes` makes AppKit allocate the pixel storage itself, and the colour space name is
            // one of AppKit's own constants.
            let rep = unsafe {
                NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bitmapFormat_bytesPerRow_bitsPerPixel(
                    NSBitmapImageRep::alloc(),
                    null_mut(),
                    source.width() as isize,
                    source.height() as isize,
                    8,
                    4,
                    true,
                    false,
                    NSDeviceRGBColorSpace,
                    NSBitmapFormat::AlphaNonpremultiplied,
                    row_len as isize,
                    32,
                )
            }
            .ok_or(PlatformError::CustomCursorFailed)?;

            let destination = rep.bitmapData();
            let stride = rep.bytesPerRow() as usize;
            if destination.is_null() || stride < row_len {
                return Err(PlatformError::CustomCursorFailed);
            }

            for (row_index, row) in source.rgba().chunks_exact(row_len).enumerate() {
                // SAFETY: the representation owns `height` rows of `bytesPerRow` bytes each, which is at least
                // `row_len` (checked above), and `row_index` is below `height` because `rgba` holds exactly
                // `height` rows of `row_len` bytes.
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        row.as_ptr(),
                        destination.add(row_index.saturating_mul(stride)),
                        row_len,
                    );
                }
            }

            rep.setSize(size);
            image.addRepresentation(&rep);
        }

        // NSCursor measures its hot spot in points from the top-left corner of the image.
        let (hotspot_x, hotspot_y) = largest.hotspot();
        let hotspot = NSPoint::new(hotspot_x, hotspot_y);
        let cursor = NSCursor::initWithImage_hotSpot(NSCursor::alloc(), &image, hotspot);

        Ok(Self { cursor })
    }

    pub fn retained(&self) -> Retained<NSCursor> {
        self.cursor.clone()
    }
}

pub struct CursorManager {
    is_inside: Cell<bool>,
    /// The system cursor being shown, or `None` while a custom cursor is.
    current: Cell<Option<MouseCursor>>,
    current_cursor: RefCell<Retained<NSCursor>>,
    empty: LazyCell<Retained<NSCursor>>,
}

impl CursorManager {
    pub fn new() -> Self {
        Self {
            current: Some(MouseCursor::Default).into(),
            current_cursor: NSCursor::arrowCursor().into(),
            empty: LazyCell::new(Self::create_empty_cursor),
            is_inside: Cell::new(false),
        }
    }

    fn create_empty_cursor() -> Retained<NSCursor> {
        let image = NSImage::initWithSize(NSImage::alloc(), NSSize::new(0.0, 0.0));
        NSCursor::initWithImage_hotSpot(NSCursor::alloc(), &image, NSPoint::ZERO)
    }

    pub fn set_is_inside(&self, is_inside: bool) {
        self.is_inside.set(is_inside);
    }

    pub fn set_cursor(&self, cursor: MouseCursor) {
        if self.current.get() == Some(cursor) {
            return;
        }

        self.current_cursor.replace(self.load(cursor.into()));
        self.current.set(Some(cursor));

        if self.is_inside.get() {
            self.update_to_current_cursor();
        }
    }

    pub fn set_custom(&self, cursor: Retained<NSCursor>) {
        self.current_cursor.replace(cursor);
        self.current.set(None);

        if self.is_inside.get() {
            self.update_to_current_cursor();
        }
    }

    pub fn update_to_current_cursor(&self) {
        //NSCursor::crosshairCursor().set();
        self.current_cursor.borrow().set();
    }

    fn load(&self, cursor: Cursor) -> Retained<NSCursor> {
        match cursor {
            Cursor::Native(loader) => loader(),
            Cursor::Undocumented(sel) => {
                let class = NSCursor::class();

                // NOTE: class.responds_to does not yield the same result (probably because NSCursor overrides respondsToSelector)
                let responds_to: bool = unsafe { msg_send![class, respondsToSelector: sel] };

                if !responds_to {
                    return NSCursor::arrowCursor();
                }

                let raw: *mut NSCursor = unsafe { class.send_message(sel, ()) };
                let cursor = unsafe { Retained::retain(raw) };

                cursor.unwrap_or_else(NSCursor::arrowCursor)
            }
            Cursor::Hidden => self.empty.retain(),
        }
    }
}

#[derive(Debug)]
enum Cursor {
    Native(fn() -> Retained<NSCursor>),
    Undocumented(Sel),
    Hidden,
}

impl From<MouseCursor> for Cursor {
    #[expect(deprecated, reason = "TODO: resize curosrs are deprecated")]
    fn from(cursor: MouseCursor) -> Self {
        match cursor {
            MouseCursor::Default => Cursor::Native(NSCursor::arrowCursor),
            MouseCursor::Hand => Cursor::Native(NSCursor::pointingHandCursor),
            MouseCursor::HandGrabbing => Cursor::Native(NSCursor::closedHandCursor),
            MouseCursor::Text => Cursor::Native(NSCursor::IBeamCursor),
            MouseCursor::VerticalText => Cursor::Native(NSCursor::IBeamCursorForVerticalLayout),
            MouseCursor::Copy => Cursor::Native(NSCursor::dragCopyCursor),
            MouseCursor::Alias => Cursor::Native(NSCursor::dragLinkCursor),
            MouseCursor::NotAllowed | MouseCursor::PtrNotAllowed => {
                Cursor::Native(NSCursor::operationNotAllowedCursor)
            }
            MouseCursor::Crosshair => Cursor::Native(NSCursor::crosshairCursor),
            MouseCursor::EResize => Cursor::Native(NSCursor::resizeRightCursor),
            MouseCursor::NResize => Cursor::Native(NSCursor::resizeUpCursor),
            MouseCursor::WResize => Cursor::Native(NSCursor::resizeLeftCursor),
            MouseCursor::SResize => Cursor::Native(NSCursor::resizeDownCursor),
            MouseCursor::EwResize | MouseCursor::ColResize => {
                Cursor::Native(NSCursor::resizeLeftRightCursor)
            }
            MouseCursor::NsResize | MouseCursor::RowResize => {
                Cursor::Native(NSCursor::resizeUpDownCursor)
            }

            MouseCursor::Help => Cursor::Undocumented(sel!(_helpCursor)),
            MouseCursor::ZoomIn => Cursor::Undocumented(sel!(_zoomInCursor)),
            MouseCursor::ZoomOut => Cursor::Undocumented(sel!(_zoomOutCursor)),
            MouseCursor::NeResize => Cursor::Undocumented(sel!(_windowResizeNorthEastCursor)),
            MouseCursor::NwResize => Cursor::Undocumented(sel!(_windowResizeNorthWestCursor)),
            MouseCursor::SeResize => Cursor::Undocumented(sel!(_windowResizeSouthEastCursor)),
            MouseCursor::SwResize => Cursor::Undocumented(sel!(_windowResizeSouthWestCursor)),
            MouseCursor::NeswResize => {
                Cursor::Undocumented(sel!(_windowResizeNorthEastSouthWestCursor))
            }
            MouseCursor::NwseResize => {
                Cursor::Undocumented(sel!(_windowResizeNorthWestSouthEastCursor))
            }

            MouseCursor::Working | MouseCursor::PtrWorking => {
                Cursor::Undocumented(sel!(busyButClickableCursor))
            }

            MouseCursor::Move => Cursor::Native(NSCursor::arrowCursor),
            MouseCursor::AllScroll => Cursor::Native(NSCursor::arrowCursor),
            MouseCursor::Cell => Cursor::Native(NSCursor::crosshairCursor),
            MouseCursor::Hidden => Cursor::Hidden,
        }
    }
}
