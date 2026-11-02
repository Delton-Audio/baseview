#[expect(clippy::exhaustive_enums, reason = "TODO: next major version")]
#[derive(Debug, Eq, PartialEq, Clone, Copy, PartialOrd, Ord, Hash, Default)]
pub enum MouseCursor {
    #[default]
    Default,
    Hand,
    HandGrabbing,
    Help,

    Hidden,

    Text,
    VerticalText,

    Working,
    PtrWorking,

    NotAllowed,
    PtrNotAllowed,

    ZoomIn,
    ZoomOut,

    Alias,
    Copy,
    Move,
    AllScroll,
    Cell,
    Crosshair,

    EResize,
    NResize,
    NeResize,
    NwResize,
    SResize,
    SeResize,
    SwResize,
    WResize,
    EwResize,
    NsResize,
    NwseResize,
    NeswResize,
    ColResize,
    RowResize,
}

/// One bitmap of a custom cursor: `width` x `height` physical pixels of straight (not premultiplied) RGBA8, rows top
/// to bottom, drawn for `scale` physical pixels per logical pixel. The hotspot is in LOGICAL pixels from the
/// top-left corner.
#[derive(Debug, Clone, PartialEq)]
pub struct CursorImage {
    width: u32,
    height: u32,
    scale: f64,
    hotspot_x: f64,
    hotspot_y: f64,
    rgba: Vec<u8>,
}

impl CursorImage {
    /// The largest side, in physical pixels, a cursor bitmap may have.
    pub const MAX_SIDE: u32 = 256;

    /// Returns `None` when `width` or `height` is 0 or above [`Self::MAX_SIDE`], when `scale` is not finite or not
    /// above 0, when `rgba.len() != width * height * 4`, or when a hotspot coordinate is not finite.
    ///
    /// A hotspot outside the image is clamped into it: x into `0.0..=(width - 1) as f64 / scale`, y into
    /// `0.0..=(height - 1) as f64 / scale`.
    pub fn new(
        width: u32, height: u32, scale: f64, hotspot_x: f64, hotspot_y: f64, rgba: Vec<u8>,
    ) -> Option<Self> {
        if width == 0 || height == 0 || width > Self::MAX_SIDE || height > Self::MAX_SIDE {
            return None;
        }
        if !scale.is_finite() || scale <= 0.0 {
            return None;
        }
        if !hotspot_x.is_finite() || !hotspot_y.is_finite() {
            return None;
        }

        let expected_len = (width as usize).checked_mul(height as usize)?.checked_mul(4)?;
        if rgba.len() != expected_len {
            return None;
        }

        let max_x = f64::from(width.saturating_sub(1)) / scale;
        let max_y = f64::from(height.saturating_sub(1)) / scale;

        Some(Self {
            width,
            height,
            scale,
            hotspot_x: hotspot_x.clamp(0.0, max_x),
            hotspot_y: hotspot_y.clamp(0.0, max_y),
            rgba,
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn scale(&self) -> f64 {
        self.scale
    }

    /// The hotspot in logical pixels, after clamping.
    pub fn hotspot(&self) -> (f64, f64) {
        (self.hotspot_x, self.hotspot_y)
    }

    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }

    /// The hotspot in this bitmap's physical pixels: each logical coordinate times `scale`, rounded to the nearest
    /// integer, clamped to `0..=width - 1` and `0..=height - 1`.
    pub fn physical_hotspot(&self) -> (u32, u32) {
        let x = (self.hotspot_x * self.scale).round() as u32;
        let y = (self.hotspot_y * self.scale).round() as u32;

        (x.min(self.width.saturating_sub(1)), y.min(self.height.saturating_sub(1)))
    }
}

/// The bitmaps of one custom cursor, one per scale.
#[derive(Debug, Clone, PartialEq)]
pub struct CursorImages {
    images: Vec<CursorImage>,
}

impl CursorImages {
    /// Returns `None` when `images` is empty or two images have the same `scale` (compared with `==`).
    pub fn new(images: Vec<CursorImage>) -> Option<Self> {
        if images.is_empty() {
            return None;
        }

        for (index, image) in images.iter().enumerate() {
            if images.iter().skip(index.saturating_add(1)).any(|other| other.scale == image.scale) {
                return None;
            }
        }

        Some(Self { images })
    }

    pub fn images(&self) -> &[CursorImage] {
        &self.images
    }

    /// The image to show at `scale`: the one whose scale is nearest (`(image.scale - scale).abs()` smallest); on a tie,
    /// the larger image scale. A `scale` that is not finite or not above 0 picks the image with scale nearest 1.0.
    pub fn for_scale(&self, scale: f64) -> &CursorImage {
        let target = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };

        let nearest = self.images.iter().reduce(|best, image| {
            let distance = (image.scale - target).abs();
            let best_distance = (best.scale - target).abs();

            if distance < best_distance || (distance == best_distance && image.scale > best.scale) {
                image
            } else {
                best
            }
        });

        match nearest {
            Some(image) => image,
            None => unreachable!("CursorImages is never empty"),
        }
    }
}

/// A cursor built from [`CursorImages`] by
/// [`WindowContext::create_custom_cursor`](crate::WindowContext::create_custom_cursor). Cheap to clone; the operating
/// system cursor is freed when the last clone is dropped. Not `Send`: create and drop it on the window's thread.
#[derive(Clone)]
pub struct CustomCursor {
    inner: std::rc::Rc<crate::platform::CustomCursor>,
}

impl CustomCursor {
    pub(crate) fn new(inner: crate::platform::CustomCursor) -> Self {
        Self { inner: std::rc::Rc::new(inner) }
    }

    pub(crate) fn platform(&self) -> &crate::platform::CustomCursor {
        &self.inner
    }
}

impl std::fmt::Debug for CustomCursor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CustomCursor { .. }")
    }
}

impl PartialEq for CustomCursor {
    fn eq(&self, other: &Self) -> bool {
        std::rc::Rc::ptr_eq(&self.inner, &other.inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixels(width: u32, height: u32) -> Vec<u8> {
        vec![0; (width as usize).saturating_mul(height as usize).saturating_mul(4)]
    }

    fn image(side: u32, scale: f64) -> CursorImage {
        match CursorImage::new(side, side, scale, 0.0, 0.0, pixels(side, side)) {
            Some(image) => image,
            None => panic!("a valid image was rejected"),
        }
    }

    fn images(scales: &[f64]) -> CursorImages {
        match CursorImages::new(scales.iter().map(|&scale| image(8, scale)).collect()) {
            Some(images) => images,
            None => panic!("valid images were rejected"),
        }
    }

    #[test]
    fn cursor_image_validates_and_clamps() {
        assert!(CursorImage::new(0, 32, 1.0, 0.0, 0.0, vec![]).is_none());
        assert!(CursorImage::new(32, 0, 1.0, 0.0, 0.0, vec![]).is_none());
        assert!(CursorImage::new(257, 1, 1.0, 0.0, 0.0, pixels(257, 1)).is_none());
        assert!(CursorImage::new(256, 1, 1.0, 0.0, 0.0, pixels(256, 1)).is_some());
        assert!(CursorImage::new(2, 2, 1.0, 0.0, 0.0, vec![0; 15]).is_none());

        for scale in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(CursorImage::new(2, 2, scale, 0.0, 0.0, pixels(2, 2)).is_none());
        }
        assert!(CursorImage::new(2, 2, 1.0, f64::NAN, 0.0, pixels(2, 2)).is_none());
        assert!(CursorImage::new(2, 2, 1.0, 0.0, f64::NAN, pixels(2, 2)).is_none());

        let clamped = CursorImage::new(32, 32, 2.0, 20.0, -1.0, pixels(32, 32));
        let Some(clamped) = clamped else { panic!("a valid image was rejected") };
        assert_eq!(clamped.hotspot(), (15.5, 0.0));
        assert_eq!(clamped.physical_hotspot(), (31, 0));

        let at_two = CursorImage::new(32, 32, 2.0, 3.0, 5.0, pixels(32, 32));
        let Some(at_two) = at_two else { panic!("a valid image was rejected") };
        assert_eq!(at_two.physical_hotspot(), (6, 10));

        // 4.5 rounds to 5 and 7.5 to 8 with `f64::round`.
        let at_one_and_a_half = CursorImage::new(48, 48, 1.5, 3.0, 5.0, pixels(48, 48));
        let Some(at_one_and_a_half) = at_one_and_a_half else {
            panic!("a valid image was rejected")
        };
        assert_eq!(at_one_and_a_half.physical_hotspot(), (5, 8));
    }

    #[test]
    fn cursor_images_needs_one_image_per_scale() {
        assert!(CursorImages::new(vec![]).is_none());
        assert!(CursorImages::new(vec![image(8, 1.0), image(16, 1.0)]).is_none());

        let both = CursorImages::new(vec![image(8, 1.0), image(16, 2.0)]);
        let Some(both) = both else { panic!("valid images were rejected") };
        assert_eq!(both.images().len(), 2);
    }

    #[test]
    fn for_scale_picks_the_nearest() {
        let set = images(&[1.0, 1.5, 2.0]);
        assert_eq!(set.for_scale(1.0).scale(), 1.0);
        assert_eq!(set.for_scale(1.2).scale(), 1.0);
        // A tie between 1.0 and 1.5 goes to the larger.
        assert_eq!(set.for_scale(1.25).scale(), 1.5);
        // A tie between 1.5 and 2.0 goes to the larger.
        assert_eq!(set.for_scale(1.75).scale(), 2.0);
        assert_eq!(set.for_scale(3.0).scale(), 2.0);
        assert_eq!(set.for_scale(0.5).scale(), 1.0);
        assert_eq!(set.for_scale(f64::NAN).scale(), 1.0);
        assert_eq!(set.for_scale(0.0).scale(), 1.0);

        let high = images(&[2.0, 3.0]);
        assert_eq!(high.for_scale(f64::NAN).scale(), 2.0);
    }
}
