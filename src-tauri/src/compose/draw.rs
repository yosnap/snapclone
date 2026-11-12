//! Draws the composition with CoreGraphics, the engine under WebKit's canvas on macOS, step
//! by step like the editor's `baseLayer`: background, the rounded card with its shadow, the
//! watermark, and the final resampling to a fixed output size.

use objc2_core_foundation::{CFAttributedString, CFBoolean, CFDictionary, CFRetained, CFString, CFType, CGAffineTransform, kCFBooleanTrue};
use objc2_core_graphics::{CGContext, CGGradient, CGGradientDrawingOptions, CGImage, CGInterpolationQuality, CGPath};
use objc2_core_text::{CTFont, CTFontUIFontType, CTLine, kCTFontAttributeName, kCTForegroundColorFromContextAttributeName};
use xcap::image::RgbaImage;

use super::balance::{canvas_read_back, dominant_border_color};
use super::canvas::{Bitmap, cg_color, premultiplied_image, srgb, straight_image};
use super::layout::{Layout, WATERMARK_FONT_SIZE, cover_rect, css_linear_gradient};
use super::style::{Background, Color, GRADIENT_PRESETS, Gradient, StoredStyle};
use super::wallpaper::BlurredWallpaper;

/// The card's drop shadow: `rgba(0, 0, 0, 0.45)`, as in the editor.
const SHADOW_COLOR: Color = Color { red: 0, green: 0, blue: 0, alpha: 115 };
/// Vertical shadow offset as a share of its blur.
const SHADOW_OFFSET: f64 = 0.4;
const WATERMARK_ALPHA: f64 = 0.8;
/// The inset colour when the capture has no edge to take it from.
const WHITE: Color = Color { red: 255, green: 255, blue: 255, alpha: 255 };
/// A glow fades to transparent white, like the editor's `rgba(255, 255, 255, 0)` stop.
const GLOW_END: Color = Color { red: 255, green: 255, blue: 255, alpha: 0 };

/// Composes `capture` (already balanced if it had to be) at its own resolution.
pub fn compose(capture: &RgbaImage, scale: f64, style: &StoredStyle, wallpaper: Option<&BlurredWallpaper>) -> Result<RgbaImage, String> {
    let (width, height) = capture.dimensions();
    let frame = Layout::new(width, height, scale, style);
    let canvas = Bitmap::new(frame.width, frame.height)?;
    paint_background(&canvas, style.background, wallpaper)?;

    // Drawing the already-rounded card with a shadow makes the shadow follow its alpha, so
    // transparent pixels (window captures) never reveal a solid backing shape.
    let fill = (frame.inset > 0).then(|| edge_color(capture));
    let capture_image = straight_image(capture)?;
    let card = rounded_card(&capture_image, width, height, &frame, fill)?;
    let (card_x, card_y, card_width, card_height) = frame.card(width, height);
    let context = canvas.context();
    CGContext::save_g_state(Some(context));
    if style.shadow > 0.0 {
        let blur = style.shadow * scale;
        // WebKit hands canvas shadows to CoreGraphics with the blur unchanged.
        CGContext::set_shadow_with_color(Some(context), Bitmap::offset(0.0, blur * SHADOW_OFFSET), blur, Some(&cg_color(SHADOW_COLOR)));
    }
    let card_rect = canvas.rect(f64::from(card_x), f64::from(card_y), f64::from(card_width), f64::from(card_height));
    CGContext::draw_image(Some(context), card_rect, Some(&card));
    CGContext::restore_g_state(Some(context));

    if let Some(text) = style.watermark_text() {
        let (right, baseline) = frame.watermark_anchor(width, height, scale);
        paint_watermark(&canvas, text, right, baseline, scale)?;
    }
    match style.ratio.output_size() {
        Some((output_width, output_height)) if (output_width, output_height) != (frame.width, frame.height) => {
            Ok(resample(&canvas, output_width, output_height)?.to_rgba())
        }
        _ => Ok(canvas.to_rgba()),
    }
}

/// The capture's predominant edge colour, which fills the inset so the frame looks like a
/// continuation of the capture's own background.
pub fn edge_color(capture: &RgbaImage) -> Color {
    dominant_border_color(&canvas_read_back(capture))
        .map_or(WHITE, |([red, green, blue, alpha], _)| Color { red, green, blue, alpha })
}

fn paint_background(canvas: &Bitmap, background: Background, wallpaper: Option<&BlurredWallpaper>) -> Result<(), String> {
    match background {
        Background::Solid(color) => {
            CGContext::set_fill_color_with_color(Some(canvas.context()), Some(&cg_color(color)));
            CGContext::fill_rect(Some(canvas.context()), canvas.bounds());
            Ok(())
        }
        Background::Gradient(index) => paint_gradient(canvas, &GRADIENT_PRESETS[index]),
        Background::None => Ok(()),
        Background::Wallpaper => match wallpaper {
            Some(wallpaper) => paint_cover(canvas, wallpaper),
            // The editor shows the first gradient until macOS hands over the picture.
            None => paint_gradient(canvas, &GRADIENT_PRESETS[0]),
        },
    }
}

fn paint_gradient(canvas: &Bitmap, gradient: &Gradient) -> Result<(), String> {
    let context = canvas.context();
    let (width, height) = (f64::from(canvas.width()), f64::from(canvas.height()));
    // A canvas gradient pads with its end colours on both sides.
    let extend = CGGradientDrawingOptions::DrawsBeforeStartLocation | CGGradientDrawingOptions::DrawsAfterEndLocation;
    let ((x0, y0), (x1, y1)) = css_linear_gradient(gradient.angle, width, height);
    let linear = cg_gradient(gradient.stops())?;
    CGContext::draw_linear_gradient(Some(context), Some(&linear), canvas.point(x0, y0), canvas.point(x1, y1), extend);
    // Like CSS `radial-gradient(circle at x y, color, transparent radius)` over the fill.
    let side = width.max(height);
    for glow in gradient.glows {
        let center = canvas.point(glow.x * width, glow.y * height);
        let radial = cg_gradient([(0.0, glow.color), (1.0, GLOW_END)].into_iter())?;
        CGContext::draw_radial_gradient(Some(context), Some(&radial), center, 0.0, center, glow.radius * side, extend);
    }
    Ok(())
}

/// Canvas gradients interpolate straight (not premultiplied) sRGB, as CGGradient does.
fn cg_gradient(stops: impl Iterator<Item = (f64, Color)>) -> Result<CFRetained<CGGradient>, String> {
    let (locations, components): (Vec<f64>, Vec<[f64; 4]>) = stops.map(|(offset, color)| (offset, color.components())).unzip();
    let components: Vec<f64> = components.concat();
    let space = srgb()?;
    // SAFETY: `components` holds four sRGB components for each of the `locations`.
    unsafe { CGGradient::with_color_components(Some(&space), components.as_ptr(), locations.as_ptr(), locations.len()) }
        .ok_or_else(|| "could not create the gradient".to_string())
}

/// The blurred desktop picture scaled to cover the canvas, smoothed at high quality.
fn paint_cover(canvas: &Bitmap, wallpaper: &BlurredWallpaper) -> Result<(), String> {
    let image = premultiplied_image(wallpaper.pixels(), wallpaper.width(), wallpaper.height())?;
    let (width, height) = (f64::from(canvas.width()), f64::from(canvas.height()));
    let (x, y, draw_width, draw_height) = cover_rect(f64::from(wallpaper.width()), f64::from(wallpaper.height()), width, height);
    let context = canvas.context();
    CGContext::save_g_state(Some(context));
    CGContext::set_interpolation_quality(Some(context), CGInterpolationQuality::High);
    CGContext::draw_image(Some(context), canvas.rect(x, y, draw_width, draw_height), Some(&image));
    CGContext::restore_g_state(Some(context));
    Ok(())
}

/// The capture inside its card: `fill` (the inset colour) around it, all clipped round.
fn rounded_card(capture: &CGImage, width: u32, height: u32, frame: &Layout, fill: Option<Color>) -> Result<CFRetained<CGImage>, String> {
    let inset = frame.inset;
    let card = Bitmap::new(width + inset * 2, height + inset * 2)?;
    let context = card.context();
    let bounds = card.bounds();
    // The layout keeps the radius within half of each side, as CoreGraphics requires.
    let radius = frame.radius.min(bounds.size.width / 2.0).min(bounds.size.height / 2.0).max(0.0);
    // SAFETY: a null transform means none; the rectangle and radii are finite and valid.
    let path = unsafe { CGPath::with_rounded_rect(bounds, radius, radius, std::ptr::null()) };
    CGContext::add_path(Some(context), Some(&path));
    CGContext::clip(Some(context));
    if let Some(fill) = fill {
        CGContext::set_fill_color_with_color(Some(context), Some(&cg_color(fill)));
        CGContext::fill_rect(Some(context), bounds);
    }
    let inset = f64::from(inset);
    CGContext::draw_image(Some(context), card.rect(inset, inset, f64::from(width), f64::from(height)), Some(capture));
    card.image()
}

/// Right-aligned at `right` on the `baseline`, in the system UI font, white at 80 % with a
/// one-point black shadow.
fn paint_watermark(canvas: &Bitmap, text: &str, right: f64, baseline: f64, scale: f64) -> Result<(), String> {
    // SAFETY: a null language means the user's; the system UI type always has a font.
    let font = unsafe { CTFont::new_ui_font_for_language(CTFontUIFontType::System, WATERMARK_FONT_SIZE * scale, None) }
        .ok_or("the system font is unavailable")?;
    let true_value: &CFBoolean = unsafe { kCFBooleanTrue }.ok_or("kCFBooleanTrue is unavailable")?;
    // SAFETY: the attribute names are CoreText constants, valid for the life of the process.
    let keys: [&CFString; 2] = unsafe { [kCTFontAttributeName, kCTForegroundColorFromContextAttributeName] };
    let values: [&CFType; 2] = [&font, true_value];
    let attributes = CFDictionary::<CFString, CFType>::from_slices(&keys, &values);
    let string = CFString::from_str(text);
    // SAFETY: the dictionary holds CoreText attribute names with values of the documented types.
    let attributed = unsafe { CFAttributedString::new(None, Some(&string), Some(attributes.as_opaque())) }
        .ok_or("could not lay out the watermark")?;
    // SAFETY: the attributed string is valid and only carries the attributes set above.
    let line = unsafe { CTLine::with_attributed_string(&attributed) };
    // SAFETY: null out-pointers skip ascent, descent and leading.
    let text_width = unsafe { line.typographic_bounds(std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut()) };

    let context = canvas.context();
    CGContext::save_g_state(Some(context));
    CGContext::set_should_smooth_fonts(Some(context), false);
    CGContext::set_alpha(Some(context), WATERMARK_ALPHA);
    CGContext::set_shadow_with_color(Some(context), Bitmap::offset(scale, scale), scale, Some(&cg_color(Color::hex(0x000000))));
    CGContext::set_fill_color_with_color(Some(context), Some(&cg_color(WHITE)));
    CGContext::set_text_matrix(Some(context), CGAffineTransform { a: 1.0, b: 0.0, c: 0.0, d: 1.0, tx: 0.0, ty: 0.0 });
    let origin = canvas.point(right - text_width, baseline);
    CGContext::set_text_position(Some(context), origin.x, origin.y);
    // SAFETY: the line and the context are valid for the duration of the call.
    unsafe { line.draw(context) };
    CGContext::restore_g_state(Some(context));
    Ok(())
}

/// The composition resampled to exactly `width` x `height`, smoothed at high quality.
fn resample(canvas: &Bitmap, width: u32, height: u32) -> Result<Bitmap, String> {
    let output = Bitmap::new(width, height)?;
    let context = output.context();
    CGContext::set_interpolation_quality(Some(context), CGInterpolationQuality::High);
    let image = canvas.image()?;
    CGContext::draw_image(Some(context), output.bounds(), Some(&image));
    Ok(output)
}
