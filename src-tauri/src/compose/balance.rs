//! "Balance": when a capture sits on a flat backdrop (a window on a plain page, a dialog on
//! a solid fill) the content is re-centred with equal margins on every side, using the
//! backdrop colour to fill the new space. Same rules as the editor's `balance.ts`.

use std::collections::HashMap;

use xcap::image::RgbaImage;

/// RGBA, alpha included.
pub type Rgba = [u8; 4];

/// Per-channel difference (alpha included) still considered "the same colour".
pub const COLOR_TOLERANCE: u8 = 12;
/// Share of border pixels that must match the predominant colour.
pub const UNIFORM_SHARE: f64 = 0.9;

/// A rectangle of pixels in the original capture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixelBox {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BalancePlan {
    pub color: Rgba,
    /// Content bounding box in the original capture.
    pub content: PixelBox,
    pub margin: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BalanceResult {
    Balanced(BalancePlan),
    NotUniform,
    NoContent,
}

pub fn analyze_balance(image: &RgbaImage) -> BalanceResult {
    let Some((color, share)) = dominant_border_color(image) else { return BalanceResult::NotUniform };
    if share < UNIFORM_SHARE {
        return BalanceResult::NotUniform;
    }
    match content_box(image, color) {
        Some(content) => BalanceResult::Balanced(balance_plan(image.width(), image.height(), content, color)),
        None => BalanceResult::NoContent,
    }
}

/// Centres `content` with the average of its four original margins on every side.
pub fn balance_plan(width: u32, height: u32, content: PixelBox, color: Rgba) -> BalancePlan {
    let left = f64::from(content.x);
    let top = f64::from(content.y);
    let right = f64::from(width - content.x - content.width);
    let bottom = f64::from(height - content.y - content.height);
    let margin = super::style::js_round((left + top + right + bottom) / 4.0) as u32;
    BalancePlan { color, content, margin, width: content.width + margin * 2, height: content.height + margin * 2 }
}

/// Most frequent colour on the outermost ring of pixels, found by bucketing to 16 levels per
/// channel and averaging the winning bucket (the first one found wins ties), plus the share
/// of ring pixels close to it.
pub fn dominant_border_color(image: &RgbaImage) -> Option<(Rgba, f64)> {
    let ring = ring_pixels(image.width(), image.height());
    if ring.is_empty() {
        return None;
    }
    // Buckets are kept in insertion order, which decides ties like iterating a JavaScript Map.
    let mut index: HashMap<u16, usize> = HashMap::new();
    let mut buckets: Vec<(u32, [u64; 4])> = Vec::new();
    for &(x, y) in &ring {
        let pixel = image.get_pixel(x, y).0;
        let key = pixel.iter().fold(0u16, |key, channel| (key << 4) | u16::from(channel >> 4));
        let slot = *index.entry(key).or_insert_with(|| {
            buckets.push((0, [0; 4]));
            buckets.len() - 1
        });
        let bucket = &mut buckets[slot];
        bucket.0 += 1;
        for (sum, channel) in bucket.1.iter_mut().zip(pixel) {
            *sum += u64::from(channel);
        }
    }
    let mut best = (0u32, [0u64; 4]);
    for bucket in buckets {
        if bucket.0 > best.0 {
            best = bucket;
        }
    }
    let color = best.1.map(|sum| super::style::js_round(sum as f64 / f64::from(best.0)) as u8);
    let matching = ring.iter().filter(|&&(x, y)| matches(image.get_pixel(x, y).0, color)).count();
    Some((color, matching as f64 / ring.len() as f64))
}

/// Bounding box of the pixels that differ from `color`, scanning inwards from each edge.
pub fn content_box(image: &RgbaImage, color: Rgba) -> Option<PixelBox> {
    let (width, height) = image.dimensions();
    let differs = |x: u32, y: u32| !matches(image.get_pixel(x, y).0, color);
    let row_has_content = |y: u32| (0..width).any(|x| differs(x, y));
    let top = (0..height).find(|&y| row_has_content(y))?;
    let bottom = (top..height).rev().find(|&y| row_has_content(y))?;
    let column_has_content = |x: u32| (top..=bottom).any(|y| differs(x, y));
    let left = (0..width).find(|&x| column_has_content(x))?;
    let right = (left..width).rev().find(|&x| column_has_content(x))?;
    Some(PixelBox { x: left, y: top, width: right - left + 1, height: bottom - top + 1 })
}

/// The re-centred capture on a backdrop of the detected colour. Content pixels are composited
/// over the backdrop with source-over, which copies them unchanged when they are opaque.
pub fn balanced_image(image: &RgbaImage, plan: &BalancePlan) -> RgbaImage {
    let mut output = RgbaImage::from_pixel(plan.width, plan.height, xcap::image::Rgba(plan.color));
    let PixelBox { x, y, width, height } = plan.content;
    for row in 0..height {
        for column in 0..width {
            let source = image.get_pixel(x + column, y + row).0;
            let target = output.get_pixel_mut(plan.margin + column, plan.margin + row);
            target.0 = source_over(source, target.0);
        }
    }
    output
}

/// Pixels as the editor reads them back from a canvas: each pixel goes through the 8-bit
/// premultiplied store, which only changes translucent ones.
pub fn canvas_read_back(image: &RgbaImage) -> std::borrow::Cow<'_, RgbaImage> {
    if image.pixels().all(|pixel| pixel.0[3] == 255) {
        return std::borrow::Cow::Borrowed(image);
    }
    let mut copy = image.clone();
    for pixel in copy.pixels_mut() {
        let alpha = u32::from(pixel.0[3]);
        for channel in &mut pixel.0[..3] {
            let premultiplied = (u32::from(*channel) * alpha + 127) / 255;
            *channel = unpremultiply(premultiplied, alpha);
        }
    }
    std::borrow::Cow::Owned(copy)
}

/// WebKit's rounding when reading premultiplied pixels back as straight alpha.
pub fn unpremultiply(channel: u32, alpha: u32) -> u8 {
    (channel * 255 + alpha / 2).checked_div(alpha).map_or(0, |value| value.min(255) as u8)
}

fn source_over(source: Rgba, target: Rgba) -> Rgba {
    let source_alpha = f64::from(source[3]) / 255.0;
    let target_alpha = f64::from(target[3]) / 255.0;
    let alpha = source_alpha + target_alpha * (1.0 - source_alpha);
    if alpha == 0.0 {
        return [0; 4];
    }
    let mut output = [0u8; 4];
    for channel in 0..3 {
        let premultiplied = f64::from(source[channel]) * source_alpha + f64::from(target[channel]) * target_alpha * (1.0 - source_alpha);
        output[channel] = (premultiplied / alpha).round().clamp(0.0, 255.0) as u8;
    }
    output[3] = (alpha * 255.0).round() as u8;
    output
}

fn matches(pixel: Rgba, color: Rgba) -> bool {
    pixel.iter().zip(color).all(|(channel, target)| channel.abs_diff(target) <= COLOR_TOLERANCE)
}

/// Every pixel on the image's outer edge, each corner counted once, in the editor's order.
fn ring_pixels(width: u32, height: u32) -> Vec<(u32, u32)> {
    if width == 0 || height == 0 {
        return Vec::new();
    }
    let mut ring: Vec<(u32, u32)> = (0..width).map(|x| (x, 0)).collect();
    if height > 1 {
        ring.extend((0..width).map(|x| (x, height - 1)));
    }
    for y in 1..height.saturating_sub(1) {
        ring.push((0, y));
        if width > 1 {
            ring.push((width - 1, y));
        }
    }
    ring
}

#[cfg(test)]
mod tests {
    use super::*;
    use xcap::image::Rgba as Pixel;

    const WHITE: Rgba = [255, 255, 255, 255];
    const RED: Rgba = [200, 30, 30, 255];

    /// A white image with a red block at (x, y, width, height).
    fn with_block(width: u32, height: u32, block: PixelBox) -> RgbaImage {
        let mut image = RgbaImage::from_pixel(width, height, Pixel(WHITE));
        for y in block.y..block.y + block.height {
            for x in block.x..block.x + block.width {
                image.put_pixel(x, y, Pixel(RED));
            }
        }
        image
    }

    #[test]
    fn off_centre_content_is_recentred_with_the_average_margin() {
        let content = PixelBox { x: 10, y: 20, width: 30, height: 10 };
        let image = with_block(100, 60, content);
        let BalanceResult::Balanced(plan) = analyze_balance(&image) else { panic!("expected a plan") };
        // Margins 10, 20, 60, 30: average 30.
        assert_eq!(plan, BalancePlan { color: WHITE, content, margin: 30, width: 90, height: 70 });
        let balanced = balanced_image(&image, &plan);
        assert_eq!(balanced.dimensions(), (90, 70));
        assert_eq!(balanced.get_pixel(30, 30).0, RED);
        assert_eq!(balanced.get_pixel(59, 39).0, RED);
        assert_eq!(balanced.get_pixel(60, 40).0, WHITE);
        assert_eq!(balanced.get_pixel(29, 29).0, WHITE);
    }

    #[test]
    fn a_busy_border_or_a_flat_image_is_not_balanced() {
        let mut busy = RgbaImage::from_pixel(10, 10, Pixel(WHITE));
        for x in 0..10 {
            busy.put_pixel(x, 0, Pixel(RED));
        }
        assert_eq!(analyze_balance(&busy), BalanceResult::NotUniform, "a quarter of the ring differs");
        assert_eq!(analyze_balance(&RgbaImage::from_pixel(8, 8, Pixel(WHITE))), BalanceResult::NoContent);
        assert_eq!(analyze_balance(&RgbaImage::new(0, 0)), BalanceResult::NotUniform);
    }

    #[test]
    fn near_colours_count_as_the_backdrop() {
        let mut image = with_block(20, 20, PixelBox { x: 5, y: 5, width: 4, height: 4 });
        image.put_pixel(0, 0, Pixel([243, 250, 255, 255]));
        let (color, share) = dominant_border_color(&image).unwrap();
        assert_eq!(color, WHITE);
        assert_eq!(share, 1.0);
        assert_eq!(content_box(&image, color), Some(PixelBox { x: 5, y: 5, width: 4, height: 4 }));
    }

    #[test]
    fn the_winning_bucket_is_averaged() {
        let mut image = RgbaImage::from_pixel(3, 3, Pixel([16, 16, 16, 255]));
        image.put_pixel(0, 0, Pixel([31, 31, 31, 255]));
        let (color, _) = dominant_border_color(&image).unwrap();
        // Eight ring pixels: seven at 16 and one at 31 share the bucket.
        assert_eq!(color, [18, 18, 18, 255], "Math.round(17.875)");
    }

    #[test]
    fn ring_counts_each_corner_once() {
        assert_eq!(ring_pixels(3, 3).len(), 8);
        assert_eq!(ring_pixels(1, 1), vec![(0, 0)]);
        assert_eq!(ring_pixels(4, 1).len(), 4);
        assert_eq!(ring_pixels(1, 4).len(), 4);
    }

    #[test]
    fn translucent_content_composites_over_the_backdrop() {
        assert_eq!(source_over([0, 0, 0, 128], WHITE), [127, 127, 127, 255]);
        assert_eq!(source_over(RED, [0, 0, 0, 0]), RED);
        assert_eq!(source_over([0, 0, 0, 0], [0, 0, 0, 0]), [0; 4]);
    }

    #[test]
    fn read_back_only_touches_translucent_pixels() {
        let opaque = RgbaImage::from_pixel(2, 2, Pixel(RED));
        assert!(matches!(canvas_read_back(&opaque), std::borrow::Cow::Borrowed(_)));
        let translucent = RgbaImage::from_pixel(1, 1, Pixel([201, 33, 7, 3]));
        let read = canvas_read_back(&translucent);
        assert_eq!(read.get_pixel(0, 0).0[3], 3);
        assert_ne!(read.get_pixel(0, 0).0, [201, 33, 7, 3], "three alpha levels cannot hold every colour");
        assert_eq!(unpremultiply(0, 0), 0);
        assert_eq!(unpremultiply(128, 128), 255);
    }
}
