use super::Result;
use crate::wrappers::win32::cursor::BitmapCursor;
use crate::CursorImages;

/// One operating system cursor per image of a [`CursorImages`], all built up front. Each is freed when this is dropped.
pub struct CustomCursor {
    images: CursorImages,
    /// The `scale` of each image in `images`, next to the cursor built from it.
    cursors: Vec<(f64, BitmapCursor)>,
}

impl CustomCursor {
    pub fn create(images: &CursorImages) -> Result<Self> {
        let mut cursors = Vec::with_capacity(images.images().len());
        for image in images.images() {
            cursors.push((image.scale(), BitmapCursor::create(image)?));
        }

        Ok(Self { images: images.clone(), cursors })
    }

    /// Makes the image for `scale` the cursor Windows shows.
    pub fn set(&self, scale: f64) {
        let picked = self.images.for_scale(scale).scale();

        if let Some((_, cursor)) =
            self.cursors.iter().find(|(image_scale, _)| *image_scale == picked)
        {
            cursor.set();
        }
    }
}
