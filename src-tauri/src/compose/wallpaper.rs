//! The desktop picture blurred once at a small size, like the editor's `blurredWallpaper`:
//! the picture is drawn on a canvas whose longer side is 480 px, 48 px past every edge, with
//! `filter: blur(24px) saturate(1.4)`. Scaling that small copy up keeps the blur
//! proportional to any output size.
//!
//! WebKit runs that canvas filter in software in sRGB: three box blurs whose width comes from
//! the standard deviation (the SVG approximation of a Gaussian, through Accelerate's box
//! convolution) and then the SVG `saturate` colour matrix. Both are reproduced here on the
//! CPU; the copy is small and made once per picture.

use std::sync::Arc;

/// Standard deviation of the blur on the reduced copy, in pixels.
const BLUR_RADIUS: f64 = 24.0;
const SATURATION: f64 = 1.4;
/// Longer side of the reduced copy.
const BLURRED_SIDE: f64 = 480.0;
/// Drawn past the edges so the blur never fades into transparency at the borders.
const BLEED: u32 = 48;

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

/// WebKit's box size for a Gaussian of `deviation`, `floor(σ · 3√(2π) / 4 + 0.5)`, made odd
/// as its Accelerate path does.
pub fn kernel_size(deviation: f64) -> usize {
    let size = ((deviation * 3.0 * (2.0 * std::f64::consts::PI).sqrt() / 4.0 + 0.5).floor() as usize).max(2);
    if size.is_multiple_of(2) { size + 1 } else { size }
}

/// Transparent space WebKit keeps around the drawing for the blur to spread into.
fn blur_outset(kernel: usize) -> u32 {
    (kernel * 3 / 2) as u32
}

/// Three box blurs of `kernel` pixels over premultiplied RGBA, each one horizontal and
/// vertical at once with edge pixels extended, rounding to the nearest level per pass.
pub fn box_blur(pixels: &mut [u8], width: usize, height: usize, kernel: usize) {
    let radius = kernel / 2;
    let area = (kernel * kernel) as u32;
    let mut rows = vec![0u32; width * height * 4];
    for _ in 0..3 {
        for y in 0..height {
            let line = &pixels[y * width * 4..(y + 1) * width * 4];
            sliding_sums(width, radius, |x, channel| u32::from(line[x * 4 + channel]), |x, channel, sum| {
                rows[(y * width + x) * 4 + channel] = sum;
            });
        }
        for x in 0..width {
            sliding_sums(height, radius, |y, channel| rows[(y * width + x) * 4 + channel], |y, channel, sum| {
                pixels[(y * width + x) * 4 + channel] = ((sum + area / 2) / area) as u8;
            });
        }
    }
}

/// Window sums of `radius` on each side along a line of `length` samples, clamping reads to
/// the line's ends, for the four channels.
fn sliding_sums(length: usize, radius: usize, read: impl Fn(usize, usize) -> u32, mut write: impl FnMut(usize, usize, u32)) {
    let last = length - 1;
    for channel in 0..4 {
        let mut sum: u32 = (0..=2 * radius).map(|offset| read((offset as isize - radius as isize).clamp(0, last as isize) as usize, channel)).sum();
        for position in 0..length {
            write(position, channel, sum);
            let entering = (position + radius + 1).min(last);
            sum = sum + read(entering, channel) - read(position.saturating_sub(radius), channel);
        }
    }
}

/// SVG `saturate(amount)` on premultiplied RGBA, applied to the unpremultiplied colour.
pub fn saturate(pixels: &mut [u8], amount: f64) {
    let s = amount;
    let matrix = [
        [0.213 + 0.787 * s, 0.715 - 0.715 * s, 0.072 - 0.072 * s],
        [0.213 - 0.213 * s, 0.715 + 0.285 * s, 0.072 - 0.072 * s],
        [0.213 - 0.213 * s, 0.715 - 0.715 * s, 0.072 + 0.928 * s],
    ];
    for pixel in pixels.as_chunks_mut::<4>().0 {
        let alpha = f64::from(pixel[3]);
        if alpha == 0.0 {
            continue;
        }
        let color = [0, 1, 2].map(|channel| f64::from(pixel[channel]) * 255.0 / alpha);
        for (channel, row) in matrix.iter().enumerate() {
            let value = (row[0] * color[0] + row[1] * color[1] + row[2] * color[2]).clamp(0.0, 255.0);
            pixel[channel] = (value * alpha / 255.0).round() as u8;
        }
    }
}

/// Blurs a decoded picture; the drawing that feeds the filter goes through CoreGraphics, so
/// it is scaled exactly like the editor's `drawImage`.
#[cfg(target_os = "macos")]
pub fn blur(image: &objc2_core_graphics::CGImage) -> Result<BlurredWallpaper, String> {
    use objc2_core_graphics::{CGContext, CGImage, CGInterpolationQuality};

    let (width, height) = blurred_size(CGImage::width(Some(image)) as u32, CGImage::height(Some(image)) as u32);
    let kernel = kernel_size(BLUR_RADIUS);
    let pad = blur_outset(kernel);
    let buffer = super::canvas::Bitmap::new(width + 2 * (BLEED + pad), height + 2 * (BLEED + pad))?;
    let context = buffer.context();
    CGContext::set_interpolation_quality(Some(context), CGInterpolationQuality::High);
    let rect = buffer.rect(f64::from(pad), f64::from(pad), f64::from(width + 2 * BLEED), f64::from(height + 2 * BLEED));
    CGContext::draw_image(Some(context), rect, Some(image));

    let (full_width, full_height) = (buffer.width() as usize, buffer.height() as usize);
    let mut pixels = buffer.premultiplied();
    box_blur(&mut pixels, full_width, full_height, kernel);
    let offset = (BLEED + pad) as usize;
    let mut cropped = Vec::with_capacity(width as usize * height as usize * 4);
    for y in offset..offset + height as usize {
        let start = (y * full_width + offset) * 4;
        cropped.extend_from_slice(&pixels[start..start + width as usize * 4]);
    }
    saturate(&mut cropped, SATURATION);
    BlurredWallpaper::from_premultiplied(width, height, cropped).ok_or_else(|| "blurred wallpaper has the wrong size".into())
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
    let decoded = crate::image_io::decode(source.bytes()).ok_or("el fondo de escritorio no se puede leer")?;
    let blurred = Arc::new(blur(&decoded)?);
    *cache = Some((source.version().to_string(), blurred.clone()));
    Ok(blurred)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_copy_is_480_on_its_longer_side() {
        assert_eq!(blurred_size(6016, 3384), (480, 270));
        assert_eq!(blurred_size(1000, 3000), (160, 480));
        assert_eq!(blurred_size(10_000, 1), (480, 1));
    }

    #[test]
    fn kernel_follows_the_svg_approximation() {
        assert_eq!(kernel_size(24.0), 45);
        // floor(1.88 + 0.5) = 2, made odd.
        assert_eq!(kernel_size(1.0), 3);
        assert_eq!(blur_outset(45), 67);
    }

    #[test]
    fn blur_preserves_flat_areas_and_spreads_a_dot() {
        let (width, height) = (9, 9);
        let mut flat = vec![100u8; width * height * 4];
        box_blur(&mut flat, width, height, 3);
        assert!(flat.iter().all(|&value| value == 100), "edges are extended, so a flat image stays flat");

        let mut dot = vec![0u8; width * height * 4];
        dot[(4 * width + 4) * 4..(4 * width + 4) * 4 + 4].copy_from_slice(&[255; 4]);
        box_blur(&mut dot, width, height, 3);
        let at = |x: usize, y: usize| dot[(y * width + x) * 4 + 3];
        assert!(at(4, 4) > at(3, 4) && at(3, 4) > at(2, 4), "falls off from the centre");
        assert_eq!(at(3, 4), at(5, 4), "symmetric");
        assert_eq!(at(4, 3), at(4, 5), "symmetric vertically");
        assert_eq!(at(0, 0), 0, "three passes of radius 1 reach three pixels at most");
    }

    #[test]
    fn saturation_pushes_colours_away_from_grey() {
        let mut pixels = vec![200, 100, 100, 255, 128, 128, 128, 255, 100, 50, 50, 128, 0, 0, 0, 0];
        saturate(&mut pixels, 1.4);
        assert!(pixels[0] > 200 && pixels[1] < 100, "red gets redder");
        assert_eq!(&pixels[4..8], &[128, 128, 128, 255], "grey stays grey");
        assert_eq!(pixels[11], 128, "alpha is untouched");
        assert!(pixels[8] <= 128, "premultiplied colour never exceeds alpha");
        assert_eq!(&pixels[12..], &[0, 0, 0, 0]);
    }

    #[test]
    fn buffers_are_validated() {
        assert!(BlurredWallpaper::from_premultiplied(2, 2, vec![0; 16]).is_some());
        assert!(BlurredWallpaper::from_premultiplied(2, 2, vec![0; 15]).is_none());
        assert!(BlurredWallpaper::from_premultiplied(0, 2, Vec::new()).is_none());
    }
}
