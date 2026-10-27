//! Captures a screen rectangle with only the windows below a given window, so the scroll
//! capture's own frame and toolbar (ordered above it) never appear in its frames, wherever
//! they sit. Ordering is used rather than `NSWindow.sharingType = .none`, which macOS no
//! longer guarantees to honour for captures.

use std::ffi::c_void;

use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use xcap::image::RgbaImage;

use crate::geometry::Rect;

/// `kCGWindowListOptionOnScreenBelowWindow`: on-screen windows below the given one only.
const ON_SCREEN_BELOW_WINDOW: u32 = 1 << 2;
/// `kCGWindowImageDefault`: full resolution, shadows as they are on screen.
const IMAGE_DEFAULT: u32 = 0;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGWindowListCreateImage(rect: CGRect, options: u32, window: u32, image_options: u32) -> *mut c_void;
    fn CGImageGetWidth(image: *mut c_void) -> usize;
    fn CGImageGetHeight(image: *mut c_void) -> usize;
    fn CGImageGetBytesPerRow(image: *mut c_void) -> usize;
    fn CGImageGetBitsPerPixel(image: *mut c_void) -> usize;
    fn CGImageGetDataProvider(image: *mut c_void) -> *mut c_void;
    fn CGDataProviderCopyData(provider: *mut c_void) -> *mut c_void;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFDataGetBytePtr(data: *mut c_void) -> *const u8;
    fn CFDataGetLength(data: *mut c_void) -> isize;
    fn CFRelease(object: *const c_void);
}

/// `rect` is in global points (origin at the top left of the main display), `window` the
/// window number everything captured must lie below.
pub fn capture(rect: Rect, window: u32) -> Result<RgbaImage, String> {
    let bounds = CGRect {
        origin: CGPoint { x: f64::from(rect.x), y: f64::from(rect.y) },
        size: CGSize { width: f64::from(rect.width), height: f64::from(rect.height) },
    };
    // SAFETY: every object created here is checked for null and released exactly once; the
    // pixel data is only read within the length CoreFoundation reports for it.
    unsafe {
        let image = CGWindowListCreateImage(bounds, ON_SCREEN_BELOW_WINDOW, window, IMAGE_DEFAULT);
        if image.is_null() {
            return Err("the screen could not be captured".into());
        }
        let result = to_rgba(image);
        CFRelease(image.cast_const());
        result
    }
}

/// Copies a 32-bit BGRA CGImage into an RGBA image, dropping any row padding.
unsafe fn to_rgba(image: *mut c_void) -> Result<RgbaImage, String> {
    // SAFETY: the caller passes a live CGImage; see `capture`.
    unsafe {
        let (width, height) = (CGImageGetWidth(image), CGImageGetHeight(image));
        let bytes_per_row = CGImageGetBytesPerRow(image);
        if CGImageGetBitsPerPixel(image) != 32 || bytes_per_row < width * 4 {
            return Err("unexpected screen pixel format".into());
        }
        let data = CGDataProviderCopyData(CGImageGetDataProvider(image));
        if data.is_null() {
            return Err("the screen pixels could not be read".into());
        }
        let length = usize::try_from(CFDataGetLength(data)).unwrap_or(0);
        let bytes = std::slice::from_raw_parts(CFDataGetBytePtr(data), length);
        let mut pixels = Vec::with_capacity(width * height * 4);
        for row in bytes.chunks_exact(bytes_per_row).take(height) {
            pixels.extend(row[..width * 4].as_chunks::<4>().0.iter().flat_map(|&[b, g, r, a]| [r, g, b, a]));
        }
        CFRelease(data.cast_const());
        RgbaImage::from_raw(width as u32, height as u32, pixels).ok_or_else(|| "the screen image is incomplete".to_string())
    }
}
