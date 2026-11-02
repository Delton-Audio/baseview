use crate::{CursorImage, MouseCursor};
use std::ffi::c_void;
use std::mem::size_of;
use std::ptr::{null_mut, NonNull};
use windows_core::{Error, Result};
use windows_sys::Win32::Graphics::Gdi::{
    CreateBitmap, CreateDIBSection, DeleteObject, BITMAPINFO, BITMAPV5HEADER, BI_BITFIELDS,
    DIB_RGB_COLORS,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateIconIndirect, DestroyIcon, LoadCursorW, SetCursor, ICONINFO,
};
use windows_sys::{
    core::PCWSTR,
    Win32::UI::WindowsAndMessaging::{
        IDC_APPSTARTING, IDC_ARROW, IDC_CROSS, IDC_HAND, IDC_HELP, IDC_IBEAM, IDC_NO, IDC_SIZEALL,
        IDC_SIZENESW, IDC_SIZENS, IDC_SIZENWSE, IDC_SIZEWE, IDC_WAIT,
    },
};

#[derive(Copy, Clone)]
pub struct SystemCursor(NonNull<c_void>);

impl SystemCursor {
    pub fn load(cursor: MouseCursor) -> Result<Self> {
        let cursor_ptr = cursor_to_lpcwstr(cursor);

        // SAFETY: the PCWSTR returned by cursor_to_lpcwstr is always a valid shared cursor ID
        let result = unsafe { LoadCursorW(null_mut(), cursor_ptr) };

        match NonNull::new(result) {
            Some(res) => Ok(Self(res)),
            None => Err(Error::from_thread()),
        }
    }

    pub fn set(&self) {
        // SAFETY: This type guarantees the HCURSOR was returned by a successful call to LoadCursorW
        unsafe { SetCursor(self.0.as_ptr()) };
    }
}

/// A cursor built from one [`CursorImage`], freed when dropped.
pub struct BitmapCursor(NonNull<c_void>);

impl BitmapCursor {
    pub fn create(image: &CursorImage) -> Result<Self> {
        let (width, height) = (image.width(), image.height());

        // SAFETY: BITMAPV5HEADER is a plain C struct of integers, for which all-zero is a valid value.
        let mut header: BITMAPV5HEADER = unsafe { std::mem::zeroed() };
        header.bV5Size = size_of::<BITMAPV5HEADER>() as u32;
        header.bV5Width = width as i32;
        // A negative height makes the bitmap top-down.
        header.bV5Height = (height as i32).wrapping_neg();
        header.bV5Planes = 1;
        header.bV5BitCount = 32;
        header.bV5Compression = BI_BITFIELDS;
        header.bV5RedMask = 0x00FF_0000;
        header.bV5GreenMask = 0x0000_FF00;
        header.bV5BlueMask = 0x0000_00FF;
        header.bV5AlphaMask = 0xFF00_0000;

        let mut bits: *mut c_void = null_mut();

        // SAFETY: the header describes a 32 bit top-down bitmap, and `bits` is a valid out-pointer. A null device
        // context is allowed with DIB_RGB_COLORS.
        let color = unsafe {
            CreateDIBSection(
                null_mut(),
                (&raw const header).cast::<BITMAPINFO>(),
                DIB_RGB_COLORS,
                &mut bits,
                null_mut(),
                0,
            )
        };
        if color.is_null() {
            return Err(Error::from_thread());
        }
        if bits.is_null() {
            // SAFETY: `color` is a bitmap this function created and nothing else uses.
            unsafe { DeleteObject(color) };
            return Err(Error::from_thread());
        }

        let len = image.rgba().len();
        // SAFETY: a successful CreateDIBSection of a width x height 32 bit bitmap points `bits` at
        // width * height * 4 bytes, which is exactly the length of `rgba` (checked by CursorImage::new). The
        // bitmap stays alive until the DeleteObject below.
        let destination = unsafe { std::slice::from_raw_parts_mut(bits.cast::<u8>(), len) };
        for (dst, src) in destination.chunks_exact_mut(4).zip(image.rgba().chunks_exact(4)) {
            // The bitmap is B, G, R, A in memory, and not premultiplied.
            if let ([db, dg, dr, da], [r, g, b, a]) = (dst, src) {
                *db = *b;
                *dg = *g;
                *dr = *r;
                *da = *a;
            }
        }

        // The AND mask is monochrome with word aligned rows. All zero, so the colour bitmap alone decides the pixels.
        let mask_len =
            (width.div_ceil(16) as usize).saturating_mul(2).saturating_mul(height as usize);
        let mask_bits = vec![0u8; mask_len];

        // SAFETY: `mask_bits` holds the width x height 1 bit bitmap that is described to CreateBitmap.
        let mask =
            unsafe { CreateBitmap(width as i32, height as i32, 1, 1, mask_bits.as_ptr().cast()) };
        if mask.is_null() {
            let error = Error::from_thread();
            // SAFETY: `color` is a bitmap this function created and nothing else uses.
            unsafe { DeleteObject(color) };
            return Err(error);
        }

        let (hotspot_x, hotspot_y) = image.physical_hotspot();
        let info = ICONINFO {
            fIcon: 0,
            xHotspot: hotspot_x,
            yHotspot: hotspot_y,
            hbmMask: mask,
            hbmColor: color,
        };

        // SAFETY: `info` refers to two live bitmaps.
        let result = unsafe { CreateIconIndirect(&info) };
        // Read the error before the DeleteObject calls below can replace it.
        let cursor = NonNull::new(result).ok_or_else(Error::from_thread);

        // SAFETY: both bitmaps were created above and nothing else uses them. The cursor keeps its own copies.
        unsafe {
            DeleteObject(mask);
            DeleteObject(color);
        }

        Ok(Self(cursor?))
    }

    pub fn set(&self) {
        // SAFETY: This type guarantees the HCURSOR was returned by a successful call to CreateIconIndirect
        unsafe { SetCursor(self.0.as_ptr()) };
    }
}

impl Drop for BitmapCursor {
    fn drop(&mut self) {
        // SAFETY: This type owns the HCURSOR, which CreateIconIndirect returned and nothing else destroys.
        unsafe { DestroyIcon(self.0.as_ptr()) };
    }
}

fn cursor_to_lpcwstr(cursor: MouseCursor) -> PCWSTR {
    match cursor {
        MouseCursor::Default => IDC_ARROW,
        MouseCursor::Hand => IDC_HAND,
        MouseCursor::HandGrabbing => IDC_SIZEALL,
        MouseCursor::Help => IDC_HELP,
        // an empty LPCWSTR results in the cursor being hidden
        MouseCursor::Hidden => std::ptr::null(),

        MouseCursor::Text => IDC_IBEAM,
        MouseCursor::VerticalText => IDC_IBEAM,

        MouseCursor::Working => IDC_WAIT,
        MouseCursor::PtrWorking => IDC_APPSTARTING,

        MouseCursor::NotAllowed => IDC_NO,
        MouseCursor::PtrNotAllowed => IDC_NO,

        MouseCursor::ZoomIn => IDC_ARROW,
        MouseCursor::ZoomOut => IDC_ARROW,

        MouseCursor::Alias => IDC_ARROW,
        MouseCursor::Copy => IDC_ARROW,
        MouseCursor::Move => IDC_SIZEALL,
        MouseCursor::AllScroll => IDC_SIZEALL,
        MouseCursor::Cell => IDC_CROSS,
        MouseCursor::Crosshair => IDC_CROSS,

        MouseCursor::EResize => IDC_SIZEWE,
        MouseCursor::NResize => IDC_SIZENS,
        MouseCursor::NeResize => IDC_SIZENESW,
        MouseCursor::NwResize => IDC_SIZENWSE,
        MouseCursor::SResize => IDC_SIZENS,
        MouseCursor::SeResize => IDC_SIZENWSE,
        MouseCursor::SwResize => IDC_SIZENESW,
        MouseCursor::WResize => IDC_SIZEWE,
        MouseCursor::EwResize => IDC_SIZEWE,
        MouseCursor::NsResize => IDC_SIZENS,
        MouseCursor::NwseResize => IDC_SIZENWSE,
        MouseCursor::NeswResize => IDC_SIZENESW,

        MouseCursor::ColResize => IDC_SIZEWE,
        MouseCursor::RowResize => IDC_SIZENS,
    }
}
