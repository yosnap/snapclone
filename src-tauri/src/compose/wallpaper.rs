//! The desktop picture reduced once to a tiny copy, like the editor's `blurredWallpaper`.
//! WebKit's canvas has no `filter` (the editor checks `"filter" in
//! CanvasRenderingContext2D.prototype`), so the editor takes its fallback: the picture drawn
//! with high-quality smoothing on a canvas whose longer side is 24 px, which scaled up
//! smoothly to cover the background stands in for the blur. If WebKit ever gains canvas
//! filters, the editor switches to `blur(24px) saturate(1.4)` on a 480 px copy and this has
//! to follow.

use std::sync::Arc;

/// Longer side of the reduced copy, the editor's `BLURRED_SIDE` without canvas filters.
const BLURRED_SIDE: f64 = 24.0;
/// Longer side the picture is decoded at before it is reduced to the tiny copy.
/// 768 px keeps the result within a level of decoding at full size (measured against the
/// editor); smaller sides drift visibly in the tiny copy.
const DECODE_SIDE: u32 = 768;

/// Premultiplied RGBA pixels, rows packed, ready to be drawn scaled to cover the canvas.
#[derive(Debug, PartialEq)]
pub struct BlurredWallpaper {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl BlurredWallpaper {
    /// `None` when the buffer does not hold `width` x `height` RGBA pixels.
    pub fn from_premultiplied(width: u32, height: u32, pixels: Vec<u8>) -> Option<Self> {
        (width > 0 && height > 0 && pixels.len() == width as usize * height as usize * 4).then_some(Self { width, height, pixels })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }
}

/// Size of the reduced copy of a `width` x `height` picture.
pub fn blurred_size(width: u32, height: u32) -> (u32, u32) {
    let factor = BLURRED_SIDE / f64::from(width.max(height).max(1));
    let side = |length: u32| (super::style::js_round(f64::from(length) * factor) as u32).max(1);
    (side(width), side(height))
}

/// Reduces the picture in an image file (any format ImageIO reads, HEIC included). ImageIO
/// first decodes it at most `decode_side` pixels wide, which keeps the first copy after a
/// wallpaper change fast; the 24 px copy is then sized from the picture's real dimensions,
/// so it rounds exactly like the editor's.
#[cfg(target_os = "macos")]
pub fn from_picture(bytes: &[u8], decode_side: u32) -> Result<BlurredWallpaper, String> {
    use objc2_core_graphics::CGImage;

    const UNREADABLE: &str = "el fondo de escritorio no se puede leer";
    let (width, height) = crate::image_io::header_size(bytes).ok_or(UNREADABLE)?;
    let decoded = crate::image_io::decode(bytes, decode_side).ok_or(UNREADABLE)?;
    // The header gives the stored size; an EXIF rotation of a quarter turn swaps it.
    let landscape = CGImage::width(Some(&decoded)) >= CGImage::height(Some(&decoded));
    let size = if landscape == (width >= height) { (width, height) } else { (height, width) };
    reduce(&decoded, size)
}

/// Reduces a decoded picture whose full, oriented size is `size` through CoreGraphics at
/// high quality, as the editor's `drawImage` does.
#[cfg(target_os = "macos")]
pub fn reduce(image: &objc2_core_graphics::CGImage, size: (u32, u32)) -> Result<BlurredWallpaper, String> {
    use objc2_core_graphics::{CGContext, CGInterpolationQuality};

    let (width, height) = blurred_size(size.0, size.1);
    let reduced = super::canvas::Bitmap::new(width, height)?;
    let context = reduced.context();
    CGContext::set_interpolation_quality(Some(context), CGInterpolationQuality::High);
    CGContext::draw_image(Some(context), reduced.bounds(), Some(image));
    BlurredWallpaper::from_premultiplied(width, height, reduced.premultiplied()).ok_or_else(|| "blurred wallpaper has the wrong size".into())
}

/// The last blurred picture, kept until the desktop picture's path or date changes.
#[derive(Default)]
pub struct BlurredWallpaperCache(std::sync::Mutex<Option<(String, Arc<BlurredWallpaper>)>>);

/// The current desktop picture, blurred. Asks AppKit on the main thread and waits for it, so
/// it must be called off the main thread. Errors are user-facing.
#[cfg(target_os = "macos")]
pub fn current(app: &tauri::AppHandle) -> Result<Arc<BlurredWallpaper>, String> {
    use tauri::Manager;

    let source = crate::wallpaper::current(app)?;
    let cache = app.state::<BlurredWallpaperCache>();
    let mut cache = cache.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some((version, blurred)) = cache.as_ref()
        && version == source.version()
    {
        return Ok(blurred.clone());
    }
    let blurred = Arc::new(from_picture(source.bytes(), DECODE_SIDE)?);
    *cache = Some((source.version().to_string(), blurred.clone()));
    Ok(blurred)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_copy_is_24_on_its_longer_side() {
        assert_eq!(blurred_size(6000, 3000), (24, 12));
        assert_eq!(blurred_size(1000, 3000), (8, 24));
        assert_eq!(blurred_size(10_000, 1), (24, 1));
    }

    #[test]
    fn buffers_are_validated() {
        assert!(BlurredWallpaper::from_premultiplied(2, 2, vec![0; 16]).is_some());
        assert!(BlurredWallpaper::from_premultiplied(2, 2, vec![0; 15]).is_none());
        assert!(BlurredWallpaper::from_premultiplied(0, 2, Vec::new()).is_none());
    }
}
