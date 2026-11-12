//! PNG writing and the small thumbnails the history view shows instead of full captures.

use std::path::{Path, PathBuf};

use xcap::image::{self, ImageEncoder, RgbaImage, codecs::png, imageops::FilterType};

/// Thumbnail width in pixels; enough for the history grid on a Retina display.
const THUMBNAIL_WIDTH: u32 = 320;

/// Fast compression: captures are re-encoded on export anyway, so speed matters more here.
pub fn write_png(path: &Path, image: &RgbaImage) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|error| error.to_string())?;
    encode_fast(std::io::BufWriter::new(file), image)
}

/// The PNG bytes of `image`, with the same fast compression, for the clipboard.
pub fn encode_png(image: &RgbaImage) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    encode_fast(&mut bytes, image)?;
    Ok(bytes)
}

fn encode_fast(writer: impl std::io::Write, image: &RgbaImage) -> Result<(), String> {
    png::PngEncoder::new_with_quality(writer, png::CompressionType::Fast, png::FilterType::Adaptive)
        .write_image(image.as_raw(), image.width(), image.height(), image::ExtendedColorType::Rgba8)
        .map_err(|error| error.to_string())
}

/// The thumbnail sits next to its capture: `<id>.png` -> `<id>.thumb.png`.
pub fn thumbnail_path(capture: &Path) -> PathBuf {
    capture.with_extension("thumb.png")
}

/// Fits `width` x `height` into `THUMBNAIL_WIDTH` keeping the aspect ratio; never upscales
/// and never collapses a side to zero.
pub fn thumbnail_size(width: u32, height: u32) -> (u32, u32) {
    if width <= THUMBNAIL_WIDTH {
        return (width.max(1), height.max(1));
    }
    let scaled = (u64::from(height) * u64::from(THUMBNAIL_WIDTH)).div_ceil(u64::from(width));
    (THUMBNAIL_WIDTH, u32::try_from(scaled).unwrap_or(u32::MAX).max(1))
}

/// Decodes a stored capture and writes its thumbnail.
pub fn write_thumbnail(capture: &Path) -> Result<(), String> {
    let source = image::open(capture).map_err(|error| error.to_string())?.into_rgba8();
    let (width, height) = thumbnail_size(source.width(), source.height());
    let thumbnail = image::imageops::resize(&source, width, height, FilterType::Triangle);
    write_png(&thumbnail_path(capture), &thumbnail)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoded_png_decodes_to_the_same_pixels() {
        let original = RgbaImage::from_fn(13, 7, |x, y| image::Rgba([(x * 19) as u8, (y * 31) as u8, 200, (x * y) as u8]));
        let bytes = encode_png(&original).unwrap();
        assert!(bytes.starts_with(b"\x89PNG"));
        assert_eq!(image::load_from_memory(&bytes).unwrap().into_rgba8(), original);
    }

    #[test]
    fn thumbnail_keeps_aspect_ratio() {
        assert_eq!(thumbnail_size(2560, 1440), (320, 180));
        assert_eq!(thumbnail_size(1920, 1081), (320, 181));
    }

    #[test]
    fn small_or_degenerate_images_are_not_upscaled_or_emptied() {
        assert_eq!(thumbnail_size(200, 100), (200, 100));
        assert_eq!(thumbnail_size(10_000, 1), (320, 1));
    }

    #[test]
    fn thumbnail_path_sits_next_to_the_capture() {
        assert_eq!(thumbnail_path(Path::new("/data/captures/abc.png")), PathBuf::from("/data/captures/abc.thumb.png"));
    }

    #[test]
    fn writes_a_thumbnail_for_a_stored_capture() {
        let dir = std::env::temp_dir().join(format!("snapclone-thumb-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let capture = dir.join("capture.png");
        write_png(&capture, &RgbaImage::from_pixel(640, 200, image::Rgba([10, 20, 30, 255]))).unwrap();
        write_thumbnail(&capture).unwrap();
        let thumbnail = image::open(thumbnail_path(&capture)).unwrap();
        assert_eq!((thumbnail.width(), thumbnail.height()), (320, 100));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
