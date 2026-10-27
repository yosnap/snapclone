//! Joins the frames of a scroll capture into one tall image. Each frame is reduced to a
//! signature (grey level averages over a few column blocks, one row each); the vertical
//! offset that best lines up the new frame's rows with the previous frame's is found by the
//! mean absolute difference, so noise and slight rendering changes are tolerated.

use xcap::image::RgbaImage;

/// Tallest image a scroll capture produces, in pixels.
pub const MAX_HEIGHT: u32 = 30_000;
/// Column blocks per row signature: enough to tell rows apart, cheap to compare.
const COLUMNS: usize = 32;
/// Mean grey level difference under which a frame counts as unchanged (a blinking caret).
const UNCHANGED_TOLERANCE: f64 = 2.0;
/// Mean grey level difference over the overlap under which an offset is trusted.
const MATCH_TOLERANCE: f64 = 10.0;
/// Fewest overlapping rows a match may rest on, and the share of the frame height they must
/// cover at least, so a few similar rows never decide the offset alone.
const MIN_OVERLAP_ROWS: usize = 16;
const MIN_OVERLAP_DIVISOR: usize = 8;

/// One row of block averages per frame row.
pub struct Signature {
    columns: usize,
    rows: usize,
    values: Vec<u8>,
}

impl Signature {
    fn row(&self, index: usize) -> &[u8] {
        &self.values[index * self.columns..(index + 1) * self.columns]
    }
}

pub fn signature(image: &RgbaImage) -> Signature {
    let (width, rows) = (image.width() as usize, image.height() as usize);
    let columns = COLUMNS.min(width.max(1));
    let mut values = Vec::with_capacity(columns * rows);
    for (_, row) in image.enumerate_rows() {
        let luma: Vec<u32> = row
            .map(|(_, _, pixel)| (u32::from(pixel[0]) * 77 + u32::from(pixel[1]) * 150 + u32::from(pixel[2]) * 29) >> 8)
            .collect();
        for column in 0..columns {
            // `columns` never exceeds the width, so every block has at least one pixel.
            let block = &luma[column * width / columns..(column + 1) * width / columns];
            values.push((block.iter().sum::<u32>() / block.len().max(1) as u32) as u8);
        }
    }
    Signature { columns, rows, values }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Match {
    /// Nothing moved; the frame adds nothing.
    Unchanged,
    /// The content moved up by this many rows.
    Offset(usize),
    /// No offset lines the frames up well enough to trust it.
    NotFound,
}

/// Mean difference between `previous` rows from `offset` on and `next` rows from the top,
/// or `None` once it is clearly above `limit` (checked as it goes, to skip hopeless offsets).
fn overlap_cost(previous: &Signature, next: &Signature, offset: usize, limit: f64) -> Option<f64> {
    let overlap = previous.rows - offset;
    let samples = (overlap * previous.columns) as f64;
    let budget = (limit * samples) as u64;
    let mut total = 0u64;
    for row in 0..overlap {
        total += previous.row(row + offset).iter().zip(next.row(row)).map(|(a, b)| u64::from(a.abs_diff(*b))).sum::<u64>();
        if total > budget {
            return None;
        }
    }
    Some(total as f64 / samples)
}

/// Finds how far the content scrolled down between two frames of the same zone.
pub fn find_offset(previous: &Signature, next: &Signature) -> Match {
    if previous.rows != next.rows || previous.columns != next.columns || previous.rows < 2 {
        return Match::NotFound;
    }
    if overlap_cost(previous, next, 0, UNCHANGED_TOLERANCE).is_some() {
        return Match::Unchanged;
    }
    let min_overlap = (previous.rows / MIN_OVERLAP_DIVISOR).max(MIN_OVERLAP_ROWS).min(previous.rows - 1);
    let mut best: Option<(usize, f64)> = None;
    for offset in 1..=previous.rows - min_overlap {
        let limit = best.map_or(MATCH_TOLERANCE, |(_, cost)| cost);
        if let Some(cost) = overlap_cost(previous, next, offset, limit)
            && best.is_none_or(|(_, best_cost)| cost < best_cost)
        {
            best = Some((offset, cost));
        }
    }
    best.map_or(Match::NotFound, |(offset, _)| Match::Offset(offset))
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// This many new rows were added at the bottom.
    Appended(u32),
    Unchanged,
    /// The frame could not be placed reliably and was left out.
    NoOverlap,
    /// The maximum height was reached; nothing more is added.
    LimitReached,
}

/// The growing image. Every frame is compared with the last one that was added, so after
/// a frame that could not be placed, scrolling back a little lets the capture continue.
pub struct Stitcher {
    width: u32,
    max_height: u32,
    height: u32,
    pixels: Vec<u8>,
    last: Signature,
    full: bool,
}

impl Stitcher {
    pub fn new(first: &RgbaImage) -> Self {
        Self::with_limit(first, MAX_HEIGHT)
    }

    fn with_limit(first: &RgbaImage, max_height: u32) -> Self {
        let height = first.height().min(max_height);
        let row_bytes = first.width() as usize * 4;
        let pixels = first.as_raw()[..row_bytes * height as usize].to_vec();
        Self { width: first.width(), max_height, height, pixels, last: signature(first), full: height >= max_height }
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn push(&mut self, frame: &RgbaImage) -> Outcome {
        if self.full {
            return Outcome::LimitReached;
        }
        if frame.width() != self.width {
            return Outcome::NoOverlap;
        }
        let next = signature(frame);
        let offset = match find_offset(&self.last, &next) {
            Match::Unchanged => return Outcome::Unchanged,
            Match::NotFound => return Outcome::NoOverlap,
            Match::Offset(offset) => offset as u32,
        };
        let added = offset.min(self.max_height - self.height);
        let row_bytes = self.width as usize * 4;
        let start = (frame.height() - offset) as usize * row_bytes;
        self.pixels.extend_from_slice(&frame.as_raw()[start..start + added as usize * row_bytes]);
        self.height += added;
        self.last = next;
        if added < offset || self.height >= self.max_height {
            self.full = true;
            return Outcome::LimitReached;
        }
        Outcome::Appended(added)
    }

    pub fn into_image(self) -> RgbaImage {
        RgbaImage::from_raw(self.width, self.height, self.pixels).expect("rows are always whole")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xcap::image::{Rgba, imageops};

    const WIDTH: u32 = 120;
    const FRAME: u32 = 200;

    /// Deterministic texture where every row differs, like text on a page.
    fn texture(x: u32, y: u32, seed: u32) -> u8 {
        let mut value = (x / 4).wrapping_mul(73_856_093) ^ y.wrapping_mul(19_349_663) ^ seed.wrapping_mul(83_492_791);
        value ^= value >> 13;
        value = value.wrapping_mul(0x5bd1_e995);
        (value ^ (value >> 15)) as u8
    }

    fn page(height: u32, seed: u32) -> RgbaImage {
        RgbaImage::from_fn(WIDTH, height, |x, y| {
            let value = texture(x, y, seed);
            Rgba([value, value.wrapping_add(40), value / 2, 255])
        })
    }

    fn frame_at(page: &RgbaImage, top: u32) -> RgbaImage {
        imageops::crop_imm(page, 0, top, WIDTH, FRAME).to_image()
    }

    /// Up to ±3 levels per channel, as compression or subpixel rendering might add.
    fn noisy(image: &RgbaImage) -> RgbaImage {
        let mut noisy = image.clone();
        for (x, y, pixel) in noisy.enumerate_pixels_mut() {
            let delta = (texture(x, y, 99) % 7) as i16 - 3;
            for channel in &mut pixel.0[..3] {
                *channel = (i16::from(*channel) + delta).clamp(0, 255) as u8;
            }
        }
        noisy
    }

    #[test]
    fn known_offsets_rebuild_the_page() {
        let source = page(600, 1);
        let mut stitcher = Stitcher::new(&frame_at(&source, 0));
        assert_eq!(stitcher.push(&frame_at(&source, 40)), Outcome::Appended(40));
        assert_eq!(stitcher.push(&frame_at(&source, 95)), Outcome::Appended(55));
        assert_eq!(stitcher.push(&frame_at(&source, 250)), Outcome::Appended(155));
        let image = stitcher.into_image();
        assert_eq!(image.height(), 450);
        assert_eq!(image, imageops::crop_imm(&source, 0, 0, WIDTH, 450).to_image());
    }

    #[test]
    fn slight_noise_still_finds_the_offset() {
        let source = page(600, 2);
        let mut stitcher = Stitcher::new(&frame_at(&source, 0));
        assert_eq!(stitcher.push(&noisy(&frame_at(&source, 63))), Outcome::Appended(63));
        assert_eq!(stitcher.push(&noisy(&frame_at(&source, 130))), Outcome::Appended(67));
        assert_eq!(stitcher.height(), 330);
    }

    #[test]
    fn identical_or_barely_changed_frames_are_skipped() {
        let source = page(600, 3);
        let first = frame_at(&source, 0);
        let mut stitcher = Stitcher::new(&first);
        assert_eq!(stitcher.push(&first), Outcome::Unchanged);
        assert_eq!(stitcher.push(&noisy(&first)), Outcome::Unchanged);
        assert_eq!(stitcher.height(), FRAME);
    }

    #[test]
    fn unrelated_frames_are_left_out_and_the_capture_can_resume() {
        let source = page(600, 4);
        let mut stitcher = Stitcher::new(&frame_at(&source, 0));
        assert_eq!(stitcher.push(&frame_at(&page(600, 5), 0)), Outcome::NoOverlap);
        // Scrolled too far: no overlap with the last frame that was added.
        assert_eq!(stitcher.push(&frame_at(&source, 300)), Outcome::NoOverlap);
        assert_eq!(stitcher.push(&frame_at(&source, 120)), Outcome::Appended(120));
        assert_eq!(stitcher.height(), 320);
    }

    #[test]
    fn height_stops_at_the_limit() {
        let source = page(600, 6);
        let mut stitcher = Stitcher::with_limit(&frame_at(&source, 0), 260);
        assert_eq!(stitcher.push(&frame_at(&source, 40)), Outcome::Appended(40));
        assert_eq!(stitcher.push(&frame_at(&source, 95)), Outcome::LimitReached);
        assert_eq!(stitcher.push(&frame_at(&source, 150)), Outcome::LimitReached);
        let image = stitcher.into_image();
        assert_eq!(image.height(), 260);
        assert_eq!(image, imageops::crop_imm(&source, 0, 0, WIDTH, 260).to_image());
    }

    #[test]
    fn plain_colour_frames_never_grow_the_image() {
        let plain = RgbaImage::from_pixel(WIDTH, FRAME, Rgba([250, 250, 250, 255]));
        let mut stitcher = Stitcher::new(&plain);
        assert_eq!(stitcher.push(&plain), Outcome::Unchanged);
        assert_eq!(find_offset(&signature(&plain), &signature(&page(FRAME, 7))), Match::NotFound);
    }
}
