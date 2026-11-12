//! Geometry of the composition, in canvas pixels with y pointing down, exactly as the
//! editor's renderer computes it. Kept free of drawing so it can be tested on any platform.

use super::style::{StoredStyle, js_round};

/// Size of the watermark text, in points.
pub const WATERMARK_FONT_SIZE: f64 = 12.0;
/// Space between the card and the watermark's text box, in points.
const WATERMARK_GAP: f64 = 10.0;
/// The watermark baseline never comes closer than this to the canvas bottom, in points.
const WATERMARK_MARGIN: f64 = 4.0;

/// Where the capture sits inside the composed canvas. `x`/`y` are the capture's origin; the
/// card is the rounded, shadowed frame around it, `inset` wider on every side.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub width: u32,
    pub height: u32,
    pub x: u32,
    pub y: u32,
    pub inset: u32,
    pub radius: f64,
}

impl Layout {
    pub fn new(image_width: u32, image_height: u32, scale: f64, style: &StoredStyle) -> Self {
        let padding = style.padding * scale;
        let inset = js_round(style.inset * scale) as u32;
        let card_width = f64::from(image_width + inset * 2);
        let card_height = f64::from(image_height + inset * 2);
        let (width, height) = fit_ratio(card_width + padding * 2.0, card_height + padding * 2.0, style.ratio.value());
        let width = js_round(width) as u32;
        let height = js_round(height) as u32;
        Self {
            width,
            height,
            x: js_round(f64::from(width - image_width) / 2.0) as u32,
            y: js_round(f64::from(height - image_height) / 2.0) as u32,
            inset,
            radius: (style.radius * scale).min(card_width / 2.0).min(card_height / 2.0),
        }
    }

    /// The card's top-left corner and size.
    pub fn card(&self, image_width: u32, image_height: u32) -> (u32, u32, u32, u32) {
        (self.x - self.inset, self.y - self.inset, image_width + self.inset * 2, image_height + self.inset * 2)
    }

    /// Right edge and baseline of the watermark: right-aligned with the card, below it,
    /// clamped inside the canvas when the padding is small.
    pub fn watermark_anchor(&self, image_width: u32, image_height: u32, scale: f64) -> (f64, f64) {
        let right = f64::from(self.x + image_width + self.inset);
        let bottom = f64::from(self.y + image_height + self.inset);
        let baseline = (bottom + (WATERMARK_GAP + WATERMARK_FONT_SIZE) * scale).min(f64::from(self.height) - WATERMARK_MARGIN * scale);
        (right, baseline)
    }
}

/// Grows the shorter side so the canvas matches the ratio; the image never shrinks.
pub fn fit_ratio(width: f64, height: f64, ratio: Option<f64>) -> (f64, f64) {
    match ratio {
        None => (width, height),
        Some(ratio) if width / height < ratio => (height * ratio, height),
        Some(ratio) => (width, width / ratio),
    }
}

/// Start and end points reproducing CSS `linear-gradient(<angle>deg, …)` over the canvas.
pub fn css_linear_gradient(angle: f64, width: f64, height: f64) -> ((f64, f64), (f64, f64)) {
    let radians = angle.to_radians();
    let dx = radians.sin();
    let dy = -radians.cos();
    let half = ((width * dx).abs() + (height * dy).abs()) / 2.0;
    let (cx, cy) = (width / 2.0, height / 2.0);
    ((cx - dx * half, cy - dy * half), (cx + dx * half, cy + dy * half))
}

/// Destination rectangle (x, y, width, height) that covers the canvas with the image,
/// centred and cropped, keeping its aspect ratio.
pub fn cover_rect(image_width: f64, image_height: f64, width: f64, height: f64) -> (f64, f64, f64, f64) {
    let scale = (width / image_width).max(height / image_height);
    let draw_width = image_width * scale;
    let draw_height = image_height * scale;
    ((width - draw_width) / 2.0, (height - draw_height) / 2.0, draw_width, draw_height)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compose::style::Ratio;

    fn style(padding: f64, inset: f64, radius: f64, ratio: Ratio) -> StoredStyle {
        StoredStyle { padding, inset, radius, ratio, ..StoredStyle::default() }
    }

    #[test]
    fn auto_ratio_adds_padding_and_inset() {
        let frame = Layout::new(800, 600, 2.0, &style(44.0, 104.0, 13.0, Ratio::Auto));
        // inset 208 px, padding 88 px on each side.
        assert_eq!(frame, Layout { width: 800 + 416 + 176, height: 600 + 416 + 176, x: 296, y: 296, inset: 208, radius: 26.0 });
        assert_eq!(frame.card(800, 600), (88, 88, 1216, 1016));
    }

    #[test]
    fn fixed_ratio_grows_the_short_side_and_centres() {
        let frame = Layout::new(100, 100, 1.0, &style(10.0, 0.0, 20.0, Ratio::parse("16:9").unwrap()));
        assert_eq!((frame.width, frame.height), (213, 120));
        assert_eq!((frame.x, frame.y), (57, 10), "Math.round(56.5) rounds up");
        let tall = Layout::new(100, 50, 1.0, &style(0.0, 0.0, 0.0, Ratio::parse("1:1").unwrap()));
        assert_eq!((tall.width, tall.height, tall.x, tall.y), (100, 100, 0, 25));
        let size = Layout::new(300, 100, 1.0, &style(0.0, 0.0, 0.0, Ratio::parse("1600x900").unwrap()));
        assert_eq!((size.width, size.height), (300, 169));
    }

    #[test]
    fn radius_and_inset_are_clamped_and_rounded() {
        let frame = Layout::new(30, 10, 1.5, &style(0.0, 3.0, 50.0, Ratio::Auto));
        assert_eq!(frame.inset, 5, "Math.round(4.5)");
        assert_eq!(frame.radius, 10.0, "at most half the card height");
    }

    #[test]
    fn fit_ratio_never_shrinks() {
        assert_eq!(fit_ratio(100.0, 50.0, None), (100.0, 50.0));
        assert_eq!(fit_ratio(100.0, 50.0, Some(1.0)), (100.0, 100.0));
        assert_eq!(fit_ratio(50.0, 100.0, Some(1.0)), (100.0, 100.0));
        assert_eq!(fit_ratio(100.0, 50.0, Some(2.0)), (100.0, 50.0));
    }

    #[test]
    fn watermark_sits_below_the_card_inside_the_canvas() {
        let frame = Layout::new(200, 100, 2.0, &style(100.0, 0.0, 0.0, Ratio::Auto));
        assert_eq!(frame.watermark_anchor(200, 100, 2.0), (400.0, 300.0 + 44.0));
        let tight = Layout::new(200, 100, 2.0, &style(5.0, 0.0, 0.0, Ratio::Auto));
        assert_eq!(tight.watermark_anchor(200, 100, 2.0).1, f64::from(tight.height) - 8.0);
    }

    #[test]
    fn linear_gradient_matches_css_geometry() {
        let close = |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9;
        // 0deg runs bottom to top, 90deg left to right, 180deg top to bottom.
        let (start, end) = css_linear_gradient(0.0, 200.0, 100.0);
        assert!(close(start, (100.0, 100.0)) && close(end, (100.0, 0.0)));
        let (start, end) = css_linear_gradient(90.0, 200.0, 100.0);
        assert!(close(start, (0.0, 50.0)) && close(end, (200.0, 50.0)));
        let (start, end) = css_linear_gradient(180.0, 200.0, 100.0);
        assert!(close(start, (100.0, 0.0)) && close(end, (100.0, 100.0)));
        // 45deg on a square reaches exactly the corners.
        let (start, end) = css_linear_gradient(45.0, 100.0, 100.0);
        assert!(close(start, (0.0, 100.0)) && close(end, (100.0, 0.0)));
    }

    #[test]
    fn cover_fills_and_centres() {
        let (x, y, width, height) = cover_rect(480.0, 270.0, 960.0, 960.0);
        assert!((x + 373.333).abs() < 1e-3 && y == 0.0 && (width - 1706.667).abs() < 1e-3 && height == 960.0);
        assert_eq!(cover_rect(100.0, 100.0, 200.0, 100.0), (0.0, -50.0, 200.0, 200.0));
    }
}
