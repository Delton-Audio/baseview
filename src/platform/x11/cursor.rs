use std::sync::{Arc, Weak};
use x11rb::connection::Connection;
use x11rb::cursor::Handle as CursorHandle;
use x11rb::protocol::render::{
    ConnectionExt as _, CreatePictureAux, PictType, Pictformat, Pictforminfo,
};
use x11rb::protocol::xproto::{ConnectionExt as _, CreateGCAux, Cursor, ImageFormat, ImageOrder};
use x11rb::xcb_ffi::XCBConnection;

use crate::platform::*;
use crate::wrappers::xlib::XlibXcbConnection;
use crate::{CursorImage, CursorImages, MouseCursor};

/// One X cursor per image of a [`CursorImages`], all built up front. Each is freed when this is dropped, if the
/// connection is still there.
pub struct CustomCursor {
    images: CursorImages,
    /// The `scale` of each image in `images`, next to the cursor built from it.
    cursors: Vec<(f64, Cursor)>,
    connection: Weak<XlibXcbConnection>,
}

impl CustomCursor {
    pub fn create(connection: &Arc<XlibXcbConnection>, images: &CursorImages) -> Result<Self> {
        let format = find_argb32_format(connection)?;

        // Held by `result` as they are made, so that a failure part way frees the cursors made so far.
        let mut result = Self {
            images: images.clone(),
            cursors: Vec::with_capacity(images.images().len()),
            connection: Arc::downgrade(connection),
        };
        for image in images.images() {
            let cursor = create_argb_cursor(connection, format, image)?;
            result.cursors.push((image.scale(), cursor));
        }

        Ok(result)
    }

    /// The X cursor for the image that suits `scale`.
    pub fn for_scale(&self, scale: f64) -> Option<Cursor> {
        let picked = self.images.for_scale(scale).scale();

        self.cursors
            .iter()
            .find(|(image_scale, _)| *image_scale == picked)
            .map(|(_, cursor)| *cursor)
    }
}

impl Drop for CustomCursor {
    fn drop(&mut self) {
        let Some(connection) = self.connection.upgrade() else { return };

        for (_, cursor) in &self.cursors {
            // Nothing to do about a failure here: the cursor goes away with the connection at the latest.
            let _ = connection.free_cursor(*cursor);
        }
    }
}

/// The RENDER picture format of depth 32 with alpha in bits 24..32 and red, green, blue below it.
fn find_argb32_format(conn: &XlibXcbConnection) -> Result<Pictformat> {
    let formats = conn.render_query_pict_formats()?.reply()?;

    let is_argb32 = |info: &&Pictforminfo| {
        let direct = &info.direct;

        info.type_ == PictType::DIRECT
            && info.depth == 32
            && direct.alpha_mask == 0xff
            && direct.alpha_shift == 24
            && direct.red_mask == 0xff
            && direct.red_shift == 16
            && direct.green_mask == 0xff
            && direct.green_shift == 8
            && direct.blue_mask == 0xff
            && direct.blue_shift == 0
    };

    formats
        .formats
        .iter()
        .find(is_argb32)
        .map(|info| info.id)
        .ok_or(PlatformError::CustomCursorUnsupported)
}

fn premultiply(channel: u8, alpha: u8) -> u8 {
    let scaled = u32::from(channel).saturating_mul(u32::from(alpha)).saturating_add(127);

    scaled.div_euclid(255) as u8
}

/// The pixels of `image` as premultiplied ARGB32 in the byte order of the server.
fn argb32_data(conn: &XlibXcbConnection, image: &CursorImage) -> Vec<u8> {
    let most_significant_first = conn.setup().image_byte_order == ImageOrder::MSB_FIRST;

    let mut data = Vec::with_capacity(image.rgba().len());
    for pixel in image.rgba().chunks_exact(4) {
        let &[r, g, b, a] = pixel else { continue };
        let (r, g, b) = (premultiply(r, a), premultiply(g, a), premultiply(b, a));

        if most_significant_first {
            data.extend_from_slice(&[a, r, g, b]);
        } else {
            data.extend_from_slice(&[b, g, r, a]);
        }
    }

    data
}

fn create_argb_cursor(
    conn: &XlibXcbConnection, format: Pictformat, image: &CursorImage,
) -> Result<Cursor> {
    let (width, height) = (image.width() as u16, image.height() as u16);
    let data = argb32_data(conn, image);

    let pixmap = conn.generate_id()?;
    let gc = conn.generate_id()?;
    let cursor = conn.generate_id()?;
    let root = conn.default_screen().root;

    conn.create_pixmap(32, pixmap, root, width, height)?;
    conn.create_gc(gc, pixmap, &CreateGCAux::new())?;

    let made = upload_cursor(conn, format, image, (pixmap, gc, cursor), &data);

    // The cursor keeps its own copy of the pixels, so the helpers go whether or not it was made.
    conn.free_gc(gc)?;
    conn.free_pixmap(pixmap)?;

    made.map(|()| cursor)
}

/// Draws `data` onto `pixmap` and makes the cursor `cursor` from it.
fn upload_cursor(
    conn: &XlibXcbConnection, format: Pictformat, image: &CursorImage,
    (pixmap, gc, cursor): (u32, u32, Cursor), data: &[u8],
) -> Result<()> {
    let (width, height) = (image.width() as u16, image.height() as u16);
    let (hotspot_x, hotspot_y) = image.physical_hotspot();
    let picture = conn.generate_id()?;

    conn.put_image(ImageFormat::Z_PIXMAP, pixmap, gc, width, height, 0, 0, 0, 32, data)?;
    conn.render_create_picture(picture, pixmap, format, &CreatePictureAux::new())?;
    let made =
        conn.render_create_cursor(cursor, picture, hotspot_x as u16, hotspot_y as u16)?.check();
    conn.render_free_picture(picture)?;

    Ok(made?)
}

fn create_empty_cursor(conn: &XlibXcbConnection) -> Result<Cursor> {
    let cursor_id = conn.generate_id()?;
    let pixmap_id = conn.generate_id()?;
    let root_window = conn.default_screen().root;
    conn.create_pixmap(1, pixmap_id, root_window, 1, 1)?;
    conn.create_cursor(cursor_id, pixmap_id, pixmap_id, 0, 0, 0, 0, 0, 0, 0, 0)?;
    conn.free_pixmap(pixmap_id)?;

    Ok(cursor_id)
}

fn load_cursor(
    conn: &XCBConnection, cursor_handle: &CursorHandle, name: &str,
) -> Result<Option<Cursor>> {
    let cursor = cursor_handle.load_cursor(conn, name)?;
    if cursor != x11rb::NONE {
        Ok(Some(cursor))
    } else {
        Ok(None)
    }
}

fn load_first_existing_cursor(
    conn: &XCBConnection, cursor_handle: &CursorHandle, names: &[&str],
) -> Result<Option<Cursor>> {
    for name in names {
        let cursor = load_cursor(conn, cursor_handle, name)?;
        if cursor.is_some() {
            return Ok(cursor);
        }
    }

    Ok(None)
}

pub(crate) fn get_xcursor(
    conn: &XlibXcbConnection, cursor_handle: &CursorHandle, cursor: MouseCursor,
) -> Result<Cursor> {
    let load = |name: &str| load_cursor(conn, cursor_handle, name);
    let loadn = |names: &[&str]| load_first_existing_cursor(conn, cursor_handle, names);

    let cursor = match cursor {
        MouseCursor::Default => None, // catch this in the fallback case below

        MouseCursor::Hand => loadn(&["hand2", "hand1"])?,
        MouseCursor::HandGrabbing => loadn(&["closedhand", "grabbing"])?,
        MouseCursor::Help => load("question_arrow")?,

        MouseCursor::Hidden => Some(create_empty_cursor(conn)?),

        MouseCursor::Text => loadn(&["text", "xterm"])?,
        MouseCursor::VerticalText => load("vertical-text")?,

        MouseCursor::Working => load("watch")?,
        MouseCursor::PtrWorking => load("left_ptr_watch")?,

        MouseCursor::NotAllowed => load("crossed_circle")?,
        MouseCursor::PtrNotAllowed => loadn(&["no-drop", "crossed_circle"])?,

        MouseCursor::ZoomIn => load("zoom-in")?,
        MouseCursor::ZoomOut => load("zoom-out")?,

        MouseCursor::Alias => load("link")?,
        MouseCursor::Copy => load("copy")?,
        MouseCursor::Move => load("move")?,
        MouseCursor::AllScroll => load("all-scroll")?,
        MouseCursor::Cell => load("plus")?,
        MouseCursor::Crosshair => load("crosshair")?,

        MouseCursor::EResize => load("right_side")?,
        MouseCursor::NResize => load("top_side")?,
        MouseCursor::NeResize => load("top_right_corner")?,
        MouseCursor::NwResize => load("top_left_corner")?,
        MouseCursor::SResize => load("bottom_side")?,
        MouseCursor::SeResize => load("bottom_right_corner")?,
        MouseCursor::SwResize => load("bottom_left_corner")?,
        MouseCursor::WResize => load("left_side")?,
        MouseCursor::EwResize => load("h_double_arrow")?,
        MouseCursor::NsResize => load("v_double_arrow")?,
        MouseCursor::NwseResize => loadn(&["bd_double_arrow", "size_bdiag"])?,
        MouseCursor::NeswResize => loadn(&["fd_double_arrow", "size_fdiag"])?,
        MouseCursor::ColResize => loadn(&["split_h", "h_double_arrow"])?,
        MouseCursor::RowResize => loadn(&["split_v", "v_double_arrow"])?,
    };

    if let Some(cursor) = cursor {
        Ok(cursor)
    } else {
        Ok(load("left_ptr")?.unwrap_or(x11rb::NONE))
    }
}
