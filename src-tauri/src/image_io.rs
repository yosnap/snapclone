//! ImageIO through direct FFI, like `capture_below`: the pixel size from an image's header
//! without decoding it, and small PNG thumbnails of formats the `image` crate cannot read
//! (the HEIC desktop pictures).

use std::ffi::c_void;

use objc2::rc::Retained;
use objc2_foundation::NSString;

type CFTypeRef = *const c_void;

/// `kCFNumberSInt64Type`.
const NUMBER_SINT64: isize = 4;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFBooleanTrue: CFTypeRef;
    static kCFTypeDictionaryKeyCallBacks: c_void;
    static kCFTypeDictionaryValueCallBacks: c_void;
    fn CFDataCreate(allocator: CFTypeRef, bytes: *const u8, length: isize) -> CFTypeRef;
    fn CFDataCreateMutable(allocator: CFTypeRef, capacity: isize) -> CFTypeRef;
    // Same signatures as in `capture_below`, which declares these too.
    fn CFDataGetBytePtr(data: *mut c_void) -> *const u8;
    fn CFDataGetLength(data: *mut c_void) -> isize;
    fn CFDictionaryCreate(
        allocator: CFTypeRef,
        keys: *const CFTypeRef,
        values: *const CFTypeRef,
        count: isize,
        key_callbacks: *const c_void,
        value_callbacks: *const c_void,
    ) -> CFTypeRef;
    fn CFDictionaryGetValue(dictionary: CFTypeRef, key: CFTypeRef) -> CFTypeRef;
    fn CFNumberCreate(allocator: CFTypeRef, kind: isize, value: *const c_void) -> CFTypeRef;
    fn CFNumberGetValue(number: CFTypeRef, kind: isize, value: *mut c_void) -> bool;
    fn CFRelease(object: CFTypeRef);
}

#[link(name = "ImageIO", kind = "framework")]
unsafe extern "C" {
    static kCGImagePropertyPixelWidth: CFTypeRef;
    static kCGImagePropertyPixelHeight: CFTypeRef;
    static kCGImageSourceCreateThumbnailFromImageAlways: CFTypeRef;
    static kCGImageSourceCreateThumbnailWithTransform: CFTypeRef;
    static kCGImageSourceThumbnailMaxPixelSize: CFTypeRef;
    fn CGImageSourceCreateWithData(data: CFTypeRef, options: CFTypeRef) -> CFTypeRef;
    fn CGImageSourceGetCount(source: CFTypeRef) -> usize;
    fn CGImageSourceCopyPropertiesAtIndex(source: CFTypeRef, index: usize, options: CFTypeRef) -> CFTypeRef;
    fn CGImageSourceCreateThumbnailAtIndex(source: CFTypeRef, index: usize, options: CFTypeRef) -> CFTypeRef;
    fn CGImageDestinationCreateWithData(data: CFTypeRef, kind: CFTypeRef, count: usize, options: CFTypeRef) -> CFTypeRef;
    fn CGImageDestinationAddImage(destination: CFTypeRef, image: CFTypeRef, properties: CFTypeRef);
    fn CGImageDestinationFinalize(destination: CFTypeRef) -> bool;
}

/// Owns one CoreFoundation reference and releases it once.
struct Owned(CFTypeRef);

impl Owned {
    fn new(object: CFTypeRef) -> Option<Self> {
        (!object.is_null()).then_some(Self(object))
    }
}

impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: the reference came from a Create or Copy function and is released only here.
        unsafe { CFRelease(self.0) }
    }
}

/// An image source over a copy of `bytes`, or `None` when ImageIO finds no image in them.
fn source(bytes: &[u8]) -> Option<Owned> {
    let length = isize::try_from(bytes.len()).ok()?;
    // SAFETY: CFDataCreate copies `length` bytes from a live slice; the source retains the data.
    unsafe {
        let data = Owned::new(CFDataCreate(std::ptr::null(), bytes.as_ptr(), length))?;
        let source = Owned::new(CGImageSourceCreateWithData(data.0, std::ptr::null()))?;
        // Asking an empty source for image 0 traps inside ImageIO, so it is never asked.
        (CGImageSourceGetCount(source.0) > 0).then_some(source)
    }
}

/// Pixel width and height of the first image, read from its header only.
pub fn header_size(bytes: &[u8]) -> Option<(u32, u32)> {
    let source = source(bytes)?;
    // SAFETY: the properties dictionary is checked for null and released by `Owned`; values
    // are only read as the CFNumbers ImageIO documents them to be.
    unsafe {
        let properties = Owned::new(CGImageSourceCopyPropertiesAtIndex(source.0, 0, std::ptr::null()))?;
        let read = |key: CFTypeRef| {
            let number = CFDictionaryGetValue(properties.0, key);
            let mut value: i64 = 0;
            let ok = !number.is_null() && CFNumberGetValue(number, NUMBER_SINT64, (&raw mut value).cast());
            ok.then(|| u32::try_from(value).ok()).flatten()
        };
        Some((read(kCGImagePropertyPixelWidth)?, read(kCGImagePropertyPixelHeight)?))
    }
}

/// A PNG at most `max_side` pixels on its longer side, with any EXIF rotation applied.
pub fn thumbnail_png(bytes: &[u8], max_side: u32) -> Result<Vec<u8>, String> {
    const FAILED: &str = "no se pudo reducir el fondo de escritorio";
    let source = source(bytes).ok_or(FAILED)?;
    let png_type: Retained<NSString> = NSString::from_str("public.png");
    let side = i64::from(max_side);
    // SAFETY: every object created here is checked for null and released by `Owned`; the
    // NSString is toll-free bridged to the CFString ImageIO expects and outlives the calls.
    unsafe {
        let size = Owned::new(CFNumberCreate(std::ptr::null(), NUMBER_SINT64, (&raw const side).cast())).ok_or(FAILED)?;
        let keys = [
            kCGImageSourceCreateThumbnailFromImageAlways,
            kCGImageSourceCreateThumbnailWithTransform,
            kCGImageSourceThumbnailMaxPixelSize,
        ];
        let values = [kCFBooleanTrue, kCFBooleanTrue, size.0];
        let options = Owned::new(CFDictionaryCreate(
            std::ptr::null(),
            keys.as_ptr(),
            values.as_ptr(),
            3,
            &raw const kCFTypeDictionaryKeyCallBacks,
            &raw const kCFTypeDictionaryValueCallBacks,
        ))
        .ok_or(FAILED)?;
        let image = Owned::new(CGImageSourceCreateThumbnailAtIndex(source.0, 0, options.0)).ok_or(FAILED)?;
        let output = Owned::new(CFDataCreateMutable(std::ptr::null(), 0)).ok_or(FAILED)?;
        let destination = Owned::new(CGImageDestinationCreateWithData(
            output.0,
            Retained::as_ptr(&png_type).cast(),
            1,
            std::ptr::null(),
        ))
        .ok_or(FAILED)?;
        CGImageDestinationAddImage(destination.0, image.0, std::ptr::null());
        if !CGImageDestinationFinalize(destination.0) {
            return Err(FAILED.into());
        }
        let length = usize::try_from(CFDataGetLength(output.0.cast_mut())).map_err(|_| FAILED)?;
        Ok(std::slice::from_raw_parts(CFDataGetBytePtr(output.0.cast_mut()), length).to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xcap::image::{Rgba, RgbaImage};

    fn sample_png(width: u32, height: u32) -> Vec<u8> {
        let path = std::env::temp_dir().join(format!("snapclone-imageio-{}.png", uuid::Uuid::new_v4()));
        crate::images::write_png(&path, &RgbaImage::from_pixel(width, height, Rgba([10, 120, 200, 255]))).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        std::fs::remove_file(path).unwrap();
        bytes
    }

    #[test]
    fn reads_the_size_without_decoding() {
        assert_eq!(header_size(&sample_png(37, 21)), Some((37, 21)));
        assert_eq!(header_size(b"not an image"), None);
        assert_eq!(header_size(&[]), None);
    }

    #[test]
    fn thumbnails_fit_the_longer_side() {
        let thumbnail = thumbnail_png(&sample_png(400, 100), 80).unwrap();
        let image = xcap::image::load_from_memory(&thumbnail).unwrap();
        assert_eq!((image.width(), image.height()), (80, 20));
        assert!(thumbnail_png(b"garbage", 80).is_err());
    }
}
