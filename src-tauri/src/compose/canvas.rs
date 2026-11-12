//! CoreGraphics bitmaps laid out like a canvas backing store: sRGB, 8 bits per channel,
//! premultiplied RGBA, first row at the top. Positions are given in canvas coordinates
//! (y pointing down) and flipped here, so callers never deal with CoreGraphics' y-up space.

use objc2_core_foundation::{CFData, CFRetained, CGFloat, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{
    CGBitmapContextCreate, CGBitmapContextCreateImage, CGBitmapContextGetBytesPerRow, CGBitmapContextGetData, CGBitmapInfo,
    CGColor, CGColorRenderingIntent, CGColorSpace, CGContext, CGDataProvider, CGImage, CGImageAlphaInfo, kCGColorSpaceSRGB,
};
use xcap::image::RgbaImage;

use super::balance::unpremultiply;
use super::style::Color;

const BYTES_PER_PIXEL: usize = 4;

pub fn srgb() -> Result<CFRetained<CGColorSpace>, String> {
    // SAFETY: the name is a CoreGraphics constant, valid for the life of the process.
    CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB })).ok_or_else(|| "sRGB colour space unavailable".to_string())
}

pub fn cg_color(color: Color) -> CFRetained<CGColor> {
    let [red, green, blue, alpha] = color.components();
    CGColor::new_srgb(red, green, blue, alpha)
}

/// A drawing surface owning its pixels; CoreGraphics allocates and frees them.
pub struct Bitmap {
    context: CFRetained<CGContext>,
    width: u32,
    height: u32,
}

impl Bitmap {
    pub fn new(width: u32, height: u32) -> Result<Self, String> {
        if width == 0 || height == 0 {
            return Err(format!("empty bitmap {width}x{height}"));
        }
        let space = srgb()?;
        // SAFETY: a null data pointer makes CoreGraphics allocate and own the buffer; the
        // format (8-bit RGBA, premultiplied last) is one CGBitmapContext supports.
        let context = unsafe {
            CGBitmapContextCreate(
                std::ptr::null_mut(),
                width as usize,
                height as usize,
                8,
                0,
                Some(&space),
                CGImageAlphaInfo::PremultipliedLast.0,
            )
        }
        .ok_or_else(|| format!("could not allocate a {width}x{height} bitmap"))?;
        Ok(Self { context, width, height })
    }

    pub fn context(&self) -> &CGContext {
        &self.context
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// A canvas-space rectangle (top-left origin) in CoreGraphics space.
    pub fn rect(&self, x: f64, y: f64, width: f64, height: f64) -> CGRect {
        CGRect::new(CGPoint::new(x, f64::from(self.height) - y - height), CGSize::new(width, height))
    }

    /// A canvas-space point in CoreGraphics space.
    pub fn point(&self, x: f64, y: f64) -> CGPoint {
        CGPoint::new(x, f64::from(self.height) - y)
    }

    /// A canvas-space offset (shadows) in CoreGraphics space.
    pub fn offset(dx: f64, dy: f64) -> CGSize {
        CGSize::new(dx, -dy)
    }

    pub fn bounds(&self) -> CGRect {
        self.rect(0.0, 0.0, f64::from(self.width), f64::from(self.height))
    }

    /// A snapshot of the current pixels.
    pub fn image(&self) -> Result<CFRetained<CGImage>, String> {
        CGBitmapContextCreateImage(Some(&self.context)).ok_or_else(|| "could not snapshot the bitmap".to_string())
    }

    /// Premultiplied pixels, rows packed without padding.
    pub fn premultiplied(&self) -> Vec<u8> {
        let row = self.width as usize * BYTES_PER_PIXEL;
        let mut pixels = Vec::with_capacity(row * self.height as usize);
        self.for_each_row(|bytes| pixels.extend_from_slice(bytes));
        pixels
    }

    /// Straight-alpha pixels, as `getImageData` or `toBlob` would return them.
    pub fn to_rgba(&self) -> RgbaImage {
        let mut pixels = Vec::with_capacity(self.width as usize * self.height as usize * BYTES_PER_PIXEL);
        self.for_each_row(|bytes| {
            for pixel in bytes.as_chunks::<BYTES_PER_PIXEL>().0 {
                let alpha = u32::from(pixel[3]);
                pixels.extend(pixel[..3].iter().map(|&channel| unpremultiply(u32::from(channel), alpha)));
                pixels.push(pixel[3]);
            }
        });
        RgbaImage::from_raw(self.width, self.height, pixels).expect("buffer matches the bitmap size")
    }

    fn for_each_row(&self, mut visit: impl FnMut(&[u8])) {
        let data = CGBitmapContextGetData(Some(&self.context)).cast::<u8>();
        let stride = CGBitmapContextGetBytesPerRow(Some(&self.context));
        let row = self.width as usize * BYTES_PER_PIXEL;
        for y in 0..self.height as usize {
            // SAFETY: the context owns `height` rows of `stride` bytes each, `stride >= row`,
            // and nothing draws into it while this shared borrow is alive.
            visit(unsafe { std::slice::from_raw_parts(data.add(y * stride), row) });
        }
    }
}

/// A CGImage over straight-alpha pixels; CoreGraphics premultiplies them when it draws,
/// as WebKit does with a decoded PNG.
pub fn straight_image(image: &RgbaImage) -> Result<CFRetained<CGImage>, String> {
    raw_image(image.as_raw(), image.width(), image.height(), CGImageAlphaInfo::Last)
}

/// A CGImage over premultiplied pixels, such as a canvas that was filtered.
pub fn premultiplied_image(pixels: &[u8], width: u32, height: u32) -> Result<CFRetained<CGImage>, String> {
    raw_image(pixels, width, height, CGImageAlphaInfo::PremultipliedLast)
}

fn raw_image(pixels: &[u8], width: u32, height: u32, alpha: CGImageAlphaInfo) -> Result<CFRetained<CGImage>, String> {
    let row = width as usize * BYTES_PER_PIXEL;
    if width == 0 || height == 0 || pixels.len() != row * height as usize {
        return Err(format!("{} bytes do not make a {width}x{height} image", pixels.len()));
    }
    // The provider keeps its own copy, so the image never points into borrowed memory.
    let data = CFData::from_bytes(pixels);
    let provider = CGDataProvider::with_cf_data(Some(&data)).ok_or("could not wrap the pixels")?;
    let space = srgb()?;
    // SAFETY: the provider holds exactly `height` rows of `row` bytes in the declared format,
    // and a null decode array means the default mapping.
    unsafe {
        CGImage::new(
            width as usize,
            height as usize,
            8,
            32,
            row,
            Some(&space),
            CGBitmapInfo(alpha.0),
            Some(&provider),
            std::ptr::null::<CGFloat>(),
            true,
            CGColorRenderingIntent::RenderingIntentDefault,
        )
    }
    .ok_or_else(|| "could not create the image".to_string())
}
