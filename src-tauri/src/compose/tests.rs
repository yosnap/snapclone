//! End-to-end compositions of small synthetic captures, one per background kind, checking
//! the layout and sampled pixels of the result.

use xcap::image::{Rgba, RgbaImage};

use super::balance::{BalanceResult, analyze_balance};
use super::style::{Background, Color, Ratio, StoredStyle};
use super::wallpaper::{BlurredWallpaper, blur, saturate};
use super::{ComposeError, compose};

/// Every pixel distinct, so an offset by one would show.
fn capture(width: u32, height: u32) -> RgbaImage {
    RgbaImage::from_fn(width, height, |x, y| Rgba([(x * 5) as u8, (y * 7) as u8, 100, 255]))
}

/// No rounding, shadow or watermark, so pixel values are exact.
fn plain(background: Background) -> StoredStyle {
    StoredStyle { background, padding: 20.0, inset: 0.0, radius: 0.0, shadow: 0.0, ..StoredStyle::default() }
}

fn close(actual: Rgba<u8>, expected: [u8; 4], tolerance: u8) -> bool {
    actual.0.iter().zip(expected).all(|(a, e)| a.abs_diff(e) <= tolerance)
}

fn assert_capture_at(output: &RgbaImage, source: &RgbaImage, x: u32, y: u32) {
    for (sx, sy) in [(0, 0), (source.width() - 1, 0), (7, 11), (source.width() - 1, source.height() - 1)] {
        assert_eq!(output.get_pixel(x + sx, y + sy), source.get_pixel(sx, sy), "capture pixel ({sx}, {sy})");
    }
}

#[test]
fn solid_background_surrounds_the_capture() {
    let source = capture(40, 30);
    let output = compose(&source, 1.0, &plain(Background::Solid(Color::hex(0x3366cc))), false, None).unwrap();
    assert_eq!(output.dimensions(), (80, 70));
    for (x, y) in [(0, 0), (79, 0), (0, 69), (79, 69), (19, 35)] {
        assert_eq!(output.get_pixel(x, y).0, [0x33, 0x66, 0xcc, 255], "background at ({x}, {y})");
    }
    assert_capture_at(&output, &source, 20, 20);
}

#[test]
fn gradient_background_spans_the_canvas_with_its_glow() {
    let source = capture(40, 30);
    let style = StoredStyle { padding: 80.0, ..plain(Background::Gradient(9)) };
    let output = compose(&source, 1.0, &style, false, None).unwrap();
    assert_eq!(output.dimensions(), (200, 190));
    // 160deg runs towards the bottom right corner, which gets the last stop exactly and is
    // beyond the glow's reach.
    assert!(close(*output.get_pixel(199, 189), [0x5f, 0xd8, 0xcc, 255], 3), "{:?}", output.get_pixel(199, 189));
    // The top left corner starts at the first stop and is lightened by the white glow.
    let corner = output.get_pixel(0, 0).0;
    assert_eq!(corner[3], 255);
    assert!(corner[0] > 0x16 + 10, "glow lightens the first stop: {corner:?}");
    assert_capture_at(&output, &source, 80, 80);
}

#[test]
fn no_background_leaves_transparent_corners() {
    let source = capture(40, 30);
    let style = StoredStyle { radius: 8.0, shadow: 10.0, ..plain(Background::None) };
    let output = compose(&source, 1.0, &style, false, None).unwrap();
    assert_eq!(output.get_pixel(0, 0).0[3], 0);
    assert_eq!(output.get_pixel(79, 0).0[3], 0);
    // The rounded corner of the card is cut away, the capture's centre is untouched.
    assert!(output.get_pixel(20, 20).0[3] < 255, "rounded corner");
    assert_eq!(output.get_pixel(40, 35), source.get_pixel(20, 15));
    // The shadow falls below the card, darker and partly opaque.
    let below = output.get_pixel(40, 52).0;
    assert!(below[3] > 0 && below[0] < 10, "shadow below the card: {below:?}");
}

#[test]
fn wallpaper_background_covers_the_canvas() {
    let picture = RgbaImage::from_pixel(64, 36, Rgba([40, 120, 200, 255]));
    let blurred = blur(&super::canvas::straight_image(&picture).unwrap()).unwrap();
    assert_eq!((blurred.width(), blurred.height()), (480, 270));
    let mut expected = vec![40, 120, 200, 255];
    saturate(&mut expected, 1.4);
    let centre = &blurred.pixels()[((135 * 480 + 240) * 4)..((135 * 480 + 240) * 4 + 4)];
    assert!(centre.iter().zip(&expected).all(|(a, e)| a.abs_diff(*e) <= 1), "{centre:?} vs {expected:?}");

    let source = capture(40, 30);
    let output = compose(&source, 1.0, &plain(Background::Wallpaper), false, Some(&blurred)).unwrap();
    let side = output.get_pixel(2, 35);
    assert!(close(*side, [expected[0], expected[1], expected[2], 255], 2), "{side:?} vs {expected:?}");
    assert_capture_at(&output, &source, 20, 20);

    // Without the picture, the first gradient shows instead.
    let fallback = compose(&source, 1.0, &plain(Background::Wallpaper), false, None).unwrap();
    assert_eq!(fallback.get_pixel(0, 0).0[3], 255);
    assert_ne!(fallback.get_pixel(0, 0), output.get_pixel(0, 0));
}

#[test]
fn inset_takes_the_edge_colour_and_scale_multiplies_lengths() {
    let mut source = RgbaImage::from_pixel(30, 20, Rgba([250, 240, 230, 255]));
    source.put_pixel(15, 10, Rgba([0, 0, 0, 255]));
    let style = StoredStyle { inset: 3.0, ..plain(Background::Solid(Color::hex(0x000000))) };
    let output = compose(&source, 2.0, &style, false, None).unwrap();
    // padding 40 and inset 6 per side at scale 2.
    assert_eq!(output.dimensions(), (30 + 12 + 80, 20 + 12 + 80));
    assert_eq!(output.get_pixel(41, 41).0, [250, 240, 230, 255], "inset filled with the edge colour");
    assert_eq!(output.get_pixel(39, 41).0, [0, 0, 0, 255], "background outside the card");
    assert_eq!(output.get_pixel(46 + 15, 46 + 10).0, [0, 0, 0, 255]);
}

#[test]
fn watermark_is_drawn_below_the_card() {
    let source = capture(120, 40);
    let style = StoredStyle { padding: 40.0, show_watermark: true, watermark: "Snapclone".into(), ..plain(Background::Solid(Color::hex(0x000000))) };
    let output = compose(&source, 1.0, &style, false, None).unwrap();
    // Text box between the card bottom (80) and the baseline (102), right-aligned at 160.
    let lit = (82..104).flat_map(|y| (100..160).map(move |x| (x, y))).filter(|&(x, y)| output.get_pixel(x, y).0[0] > 100).count();
    assert!(lit > 20, "white text pixels found: {lit}");
    let left = (82..104).flat_map(|y| (0..40).map(move |x| (x, y))).filter(|&(x, y)| output.get_pixel(x, y).0[0] > 0).count();
    assert_eq!(left, 0, "nothing left of the card's left edge");
}

#[test]
fn fixed_output_size_is_resampled() {
    let source = capture(40, 30);
    let style = StoredStyle { padding: 10.0, ratio: Ratio::parse("64x36").unwrap(), ..plain(Background::Solid(Color::hex(0x3366cc))) };
    let output = compose(&source, 1.0, &style, false, None).unwrap();
    assert_eq!(output.dimensions(), (64, 36));
    assert!(close(*output.get_pixel(0, 0), [0x33, 0x66, 0xcc, 255], 1));
}

#[test]
fn balance_recentres_before_composing() {
    let mut source = RgbaImage::from_pixel(100, 60, Rgba([255, 255, 255, 255]));
    for y in 20..30 {
        for x in 10..40 {
            source.put_pixel(x, y, Rgba([200, 30, 30, 255]));
        }
    }
    let BalanceResult::Balanced(plan) = analyze_balance(&source) else { panic!("expected a plan") };
    let style = plain(Background::Solid(Color::hex(0x000000)));
    let output = compose(&source, 1.0, &style, true, None).unwrap();
    assert_eq!(output.dimensions(), (plan.width + 40, plan.height + 40));
    assert_eq!(output.get_pixel(20 + plan.margin, 20 + plan.margin).0, [200, 30, 30, 255]);
    let unbalanced = compose(&source, 1.0, &style, false, None).unwrap();
    assert_eq!(unbalanced.dimensions(), (140, 100));
    // A capture that cannot be balanced is composed as it is.
    let flat = RgbaImage::from_pixel(10, 10, Rgba([1, 2, 3, 255]));
    assert_eq!(compose(&flat, 1.0, &style, true, None).unwrap().dimensions(), (50, 50));
}

#[test]
fn invalid_input_is_rejected() {
    let style = StoredStyle::default();
    assert!(matches!(compose(&RgbaImage::new(0, 5), 1.0, &style, false, None), Err(ComposeError::Failed(_))));
    assert!(matches!(compose(&capture(4, 4), f64::NAN, &style, false, None), Err(ComposeError::Failed(_))));
    assert!(BlurredWallpaper::from_premultiplied(1, 1, vec![0; 3]).is_none());
}
