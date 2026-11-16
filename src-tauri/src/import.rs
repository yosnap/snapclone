//! Images that were not captured here: the pasteboard, a file picked in the open dialog or
//! one dropped on the editor. Each one becomes a PNG in `captures/` and follows the normal
//! capture flow (history, thumbnail, `capture-ready`). Paths only ever come from the dialog
//! or the system's drop event, never from the webview.

use std::{
    io::Cursor,
    path::{Path, PathBuf},
};

use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;
use xcap::image::ImageReader;

use crate::{capture, editor_window};

const FILE_FORMATS: &[Format] = &[Format::Png, Format::Jpeg];
const FILE_UNSUPPORTED: &str = "Solo se pueden abrir imágenes PNG o JPEG";
/// Far above any screenshot; stops a mistaken drop from reading gigabytes into memory.
pub(crate) const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;

/// No display goes beyond 3x; a print-resolution `pHYs` must not blow up the styling.
const MAX_SCALE: f64 = 3.0;
/// WebKit refuses canvases beyond this area, so a larger image could never be edited.
const MAX_PIXELS: u64 = 16_384 * 16_384;
const MAX_SIDE: u32 = 32_767;

/// Encodings recognised by their first bytes, never by a file name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Png,
    Jpeg,
    Tiff,
}

pub fn sniff(bytes: &[u8]) -> Option<Format> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(Format::Png)
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(Format::Jpeg)
    } else if bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*") {
        Some(Format::Tiff)
    } else {
        None
    }
}

/// Turns an accepted image into PNG bytes plus the scale to style it with. PNGs are kept as
/// they are, so their colour profile and `pHYs` survive; other formats are re-encoded. The
/// size is checked from the header first, so an oversized image is refused before any
/// decoding allocates its pixels.
pub fn prepare(bytes: Vec<u8>, accepted: &[Format], unsupported: &str) -> Result<(Vec<u8>, f64), String> {
    let format = sniff(&bytes).filter(|format| accepted.contains(format)).ok_or(unsupported)?;
    let (width, height) = native::header_size(&bytes).ok_or("La imagen está dañada o no se puede leer")?;
    check_size(width, height)?;
    let png = match format {
        Format::Png => bytes,
        Format::Jpeg | Format::Tiff => native::to_png(&bytes)?,
    };
    // The converted PNG is checked again: its header is what the editor will decode.
    let (width, height) = ImageReader::new(Cursor::new(&png))
        .with_guessed_format()
        .map_err(|error| error.to_string())?
        .into_dimensions()
        .map_err(|_| "La imagen está dañada o no se puede leer".to_string())?;
    check_size(width, height)?;
    let scale = capture::png_scale(&png).min(MAX_SCALE);
    Ok((png, scale))
}

pub(crate) fn check_size(width: u32, height: u32) -> Result<(), String> {
    if width == 0 || height == 0 {
        return Err("La imagen está vacía".into());
    }
    if width > MAX_SIDE || height > MAX_SIDE || u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err(format!("La imagen es demasiado grande para editarla ({width} × {height} px)"));
    }
    Ok(())
}

/// Decoder limits for images this app accepts but `image`'s own default `max_alloc` (512MiB)
/// would refuse: a legitimate capture up to `MAX_PIXELS` needs close to 1GiB of RGBA. Shared by
/// every decode path (clipboard copy/paste, file conversion) so the cap stays in one place.
pub(crate) fn decode_limits() -> xcap::image::Limits {
    // `Limits` is `#[non_exhaustive]`: built from `default()`, then its public fields are set.
    let mut limits = xcap::image::Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(MAX_PIXELS * 4 + 256 * 1024 * 1024);
    limits
}

/// Reads only the declared dimensions, without decoding pixels. `native` stays private; this is
/// the one thing another module (`wallpaper`'s Windows thumbnail) needs from it, to check an
/// untrusted local image's size before decoding it, the same way `prepare` does.
pub(crate) fn header_size(bytes: &[u8]) -> Option<(u32, u32)> {
    native::header_size(bytes)
}

/// Writes the PNG to `captures/` and hands it to the editor like a fresh capture.
fn store(app: &AppHandle, png: &[u8], scale: f64) -> Result<(), String> {
    let id = uuid::Uuid::new_v4().to_string();
    let path = capture::capture_dir(app)?.join(format!("{id}.png"));
    std::fs::write(&path, png).map_err(|error| format!("No se pudo guardar la imagen: {error}"))?;
    capture::deliver(app, id, path, scale);
    Ok(())
}

/// Loads, converts and stores off the main thread; failures end up in the editor's status line.
fn open_with(
    app: &AppHandle,
    load: impl FnOnce() -> Result<Vec<u8>, String> + Send + 'static,
    accepted: &'static [Format],
    unsupported: &'static str,
) {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let result = load()
            .and_then(|bytes| prepare(bytes, accepted, unsupported))
            .and_then(|(png, scale)| store(&app, &png, scale));
        if let Err(error) = result {
            editor_window::notify(&app, &error, true);
        }
    });
}

fn read_file(path: &Path) -> Result<Vec<u8>, String> {
    let metadata = std::fs::metadata(path).map_err(|error| format!("No se pudo abrir el archivo: {error}"))?;
    if !metadata.is_file() {
        return Err(FILE_UNSUPPORTED.into());
    }
    if metadata.len() > MAX_FILE_BYTES {
        return Err("El archivo es demasiado grande para ser una captura".into());
    }
    std::fs::read(path).map_err(|error| format!("No se pudo leer el archivo: {error}"))
}

fn open_path(app: &AppHandle, path: PathBuf) {
    open_with(app, move || read_file(&path), FILE_FORMATS, FILE_UNSUPPORTED);
}

/// Shows the system open dialog as a sheet of the editor, which is brought forward first so
/// the dialog never opens behind other apps when started from the menu bar.
pub fn open_file_dialog(app: &AppHandle) {
    editor_window::show(app, false);
    let mut dialog = app.dialog().file().set_title("Abrir imagen").add_filter("Imagen PNG o JPEG", &["png", "jpg", "jpeg"]);
    if let Some(window) = app.get_webview_window(capture::EDITOR_WINDOW) {
        dialog = dialog.set_parent(&window);
    }
    let app = app.clone();
    dialog.pick_file(move |chosen| {
        let Some(chosen) = chosen else { return };
        match chosen.into_path() {
            Ok(path) => open_path(&app, path),
            Err(error) => editor_window::notify(&app, &format!("No se pudo abrir el archivo: {error}"), true),
        }
    });
}

/// Files dropped on the editor; the system gives the paths. Only the first one is opened,
/// since the editor shows one capture at a time.
pub fn open_dropped(app: &AppHandle, paths: &[PathBuf]) {
    let Some(path) = paths.first() else { return };
    #[cfg(windows)]
    if is_own_export(app, path) {
        // A "flick" too quick for `drag_export` to see the button still down starts
        // `DoDragDrop` with it already released, so the drop lands back on the very window
        // that started the drag — never treat that as a real import (Windows only).
        return;
    }
    open_path(app, path.clone());
}

/// Whether `path` is inside this app's own export folder (`output::export_root`) — the only way
/// a dropped path can land there is a drag the app itself just started.
#[cfg(windows)]
fn is_own_export(app: &AppHandle, path: &Path) -> bool {
    crate::output::export_root(app).is_ok_and(|root| path.starts_with(root))
}

#[tauri::command]
pub fn open_image_file(app: AppHandle) {
    open_file_dialog(&app);
}

/// Opens the image on the pasteboard. AppKit is read here, so call it on the main thread
/// (menu events and synchronous commands run there).
#[cfg(target_os = "macos")]
pub fn open_clipboard(app: &AppHandle) {
    match native::pasteboard_image() {
        Some(bytes) => open_with(app, move || Ok(bytes), &[Format::Png, Format::Tiff], "El portapapeles no tiene una imagen PNG o TIFF"),
        None => editor_window::notify(app, "El portapapeles no tiene ninguna imagen", true),
    }
}

/// Opens the image on the pasteboard. Unlike AppKit, nothing here needs the main thread: the
/// whole read (and, on Windows, the DIB→PNG conversion) runs inside `open_with`'s own
/// `spawn_blocking`.
#[cfg(not(target_os = "macos"))]
pub fn open_clipboard(app: &AppHandle) {
    open_with(app, native::pasteboard_image, &[Format::Png], "El portapapeles no tiene una imagen que se pueda abrir");
}

/// ⌘V in the editor. Synchronous, so it runs on the main thread like AppKit expects.
#[tauri::command]
pub fn open_clipboard_image(app: AppHandle) {
    open_clipboard(&app);
}

#[cfg(target_os = "macos")]
mod native {
    use objc2_app_kit::{
        NSBitmapImageFileType, NSBitmapImageRep, NSPasteboard, NSPasteboardTypePNG, NSPasteboardTypeTIFF,
    };
    use objc2_foundation::{NSData, NSDictionary};

    pub use crate::image_io::header_size;

    /// PNG first: it is what screenshots copy and needs no conversion. Apps that only copy
    /// TIFF (Preview, some browsers) still work.
    pub fn pasteboard_image() -> Option<Vec<u8>> {
        let pasteboard = NSPasteboard::generalPasteboard();
        // SAFETY: AppKit defines these constants for the lifetime of the process.
        let types = unsafe { [NSPasteboardTypePNG, NSPasteboardTypeTIFF] };
        types.into_iter().find_map(|kind| pasteboard.dataForType(kind)).map(|data| data.to_vec())
    }

    /// Re-encodes JPEG or TIFF as PNG with ImageIO, which also keeps the resolution, so a
    /// Retina TIFF comes out with its 144 DPI `pHYs`.
    pub fn to_png(bytes: &[u8]) -> Result<Vec<u8>, String> {
        objc2::rc::autoreleasepool(|_| {
            let data = NSData::with_bytes(bytes);
            let image = NSBitmapImageRep::imageRepWithData(&data).ok_or("La imagen está dañada o no se puede leer")?;
            // SAFETY: an empty properties dictionary is valid for every file type.
            let png = unsafe { image.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new()) };
            png.map(|data| data.to_vec()).ok_or_else(|| "No se pudo convertir la imagen a PNG".to_string())
        })
    }

    /// Encodes an image in another format; only the tests need it, to build JPEG and TIFF input.
    #[cfg(test)]
    pub fn encode(png: &[u8], kind: NSBitmapImageFileType) -> Vec<u8> {
        let image = NSBitmapImageRep::imageRepWithData(&NSData::with_bytes(png)).expect("valid PNG");
        // SAFETY: an empty properties dictionary is valid for every file type.
        unsafe { image.representationUsingType_properties(kind, &NSDictionary::new()) }.expect("encodes").to_vec()
    }
}

#[cfg(target_os = "windows")]
mod native {
    use std::time::Duration;

    use windows::{
        Win32::Foundation::{HANDLE, HGLOBAL},
        Win32::Graphics::Gdi::{BI_BITFIELDS, BI_RGB},
        Win32::System::DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard, RegisterClipboardFormatW},
        Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock},
        Win32::System::Ole::{CF_DIB, CF_DIBV5},
        core::HSTRING,
    };
    use xcap::image::{ExtendedColorType, ImageEncoder, codecs::png::PngEncoder};

    /// Reads only the declared dimensions, never decoding pixels — the first line of defence
    /// against a compression bomb, before `to_png` allocates anything (OWASP A10:2025).
    pub fn header_size(bytes: &[u8]) -> Option<(u32, u32)> {
        xcap::image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok()?.into_dimensions().ok()
    }

    /// `decode_limits()` is defence in depth on top of `header_size`: it makes the decoder
    /// itself reject a header that lied about the real image, and raises the default
    /// `max_alloc` so a large-but-valid image does not fail for a reason the user never sees.
    pub fn to_png(bytes: &[u8]) -> Result<Vec<u8>, String> {
        let mut reader =
            xcap::image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().map_err(|error| error.to_string())?;
        reader.limits(super::decode_limits());
        let image = reader.decode().map_err(|error| error.to_string())?;
        let mut png = Vec::new();
        image
            .write_to(&mut std::io::Cursor::new(&mut png), xcap::image::ImageFormat::Png)
            .map_err(|error| error.to_string())?;
        Ok(png)
    }

    /// Encodes a PNG in another format; only the tests need it, to build JPEG input.
    #[cfg(test)]
    pub fn encode(png: &[u8], format: xcap::image::ImageFormat) -> Vec<u8> {
        let image = xcap::image::load_from_memory(png).expect("valid PNG");
        let mut out = Vec::new();
        image.write_to(&mut std::io::Cursor::new(&mut out), format).expect("encodes");
        out
    }

    const BAD_IMAGE: &str = "La imagen está dañada o no se puede leer";
    const UNSUPPORTED_DIB: &str = "El portapapeles tiene una imagen en un formato que no se puede abrir";

    /// Holds the clipboard open for this thread and guarantees it is closed again, even on an
    /// early return. `OpenClipboard` is only tied to the thread that calls it, not the main one.
    struct ClipboardGuard;

    impl ClipboardGuard {
        /// A couple of short retries: another app can briefly be holding the clipboard too.
        fn open() -> Result<Self, String> {
            for attempt in 0..3 {
                // SAFETY: no window needs to own the clipboard just to read from it.
                if unsafe { OpenClipboard(None) }.is_ok() {
                    return Ok(Self);
                }
                if attempt < 2 {
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
            Err("No se pudo leer el portapapeles".into())
        }
    }

    impl Drop for ClipboardGuard {
        fn drop(&mut self) {
            // SAFETY: only reached after a successful `OpenClipboard` on this same thread.
            let _ = unsafe { CloseClipboard() };
        }
    }

    /// A locked view of a clipboard global block. Never copies more than the caller asks for,
    /// so the size can be checked from a short header before any full-size allocation happens.
    struct GlobalBytes {
        handle: HGLOBAL,
        ptr: *const u8,
        len: usize,
    }

    impl GlobalBytes {
        /// `handle` must come straight from `GetClipboardData` while the clipboard is open; it
        /// stays owned by the clipboard and must never be freed here.
        fn lock(handle: HANDLE) -> Result<Self, String> {
            let handle = HGLOBAL(handle.0);
            // SAFETY: `handle` is a live global block owned by the open clipboard.
            let len = unsafe { GlobalSize(handle) };
            // SAFETY: same block; locking it for reading is always valid while it is owned.
            let ptr = unsafe { GlobalLock(handle) };
            if ptr.is_null() {
                return Err("No se pudo leer el portapapeles".into());
            }
            Ok(Self { handle, ptr: ptr as *const u8, len })
        }

        fn head(&self, len: usize) -> Result<&[u8], String> {
            if self.len < len {
                return Err(BAD_IMAGE.into());
            }
            // SAFETY: `len` was just checked against the block's own reported size.
            Ok(unsafe { std::slice::from_raw_parts(self.ptr, len) })
        }

        fn check_len(&self, max: u64) -> Result<(), String> {
            if self.len as u64 > max {
                return Err("La imagen es demasiado grande para editarla".into());
            }
            Ok(())
        }

        fn to_vec(&self) -> Vec<u8> {
            // SAFETY: `self.len` is exactly the size Windows reported for this block.
            unsafe { std::slice::from_raw_parts(self.ptr, self.len) }.to_vec()
        }
    }

    impl Drop for GlobalBytes {
        fn drop(&mut self) {
            // SAFETY: `self.handle` was locked exactly once by `lock`, matched by this unlock.
            let _ = unsafe { GlobalUnlock(self.handle) };
        }
    }

    fn clipboard_data(format: u32) -> Option<HANDLE> {
        // SAFETY: only called while a `ClipboardGuard` holds the clipboard open.
        unsafe { GetClipboardData(format) }.ok()
    }

    /// The format name browsers, Recortes and `arboard` all register PNG under.
    fn png_format() -> u32 {
        // SAFETY: a plain format-name lookup; registers the name on first use, like any app.
        unsafe { RegisterClipboardFormatW(&HSTRING::from("PNG")) }
    }

    fn png_ihdr_size(head: &[u8]) -> Result<(u32, u32), String> {
        const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
        if head.len() < 24 || !head.starts_with(SIGNATURE) || &head[12..16] != b"IHDR" {
            return Err(BAD_IMAGE.into());
        }
        let width = u32::from_be_bytes(head[16..20].try_into().unwrap());
        let height = u32::from_be_bytes(head[20..24].try_into().unwrap());
        Ok((width, height))
    }

    /// The `BITMAPINFOHEADER`/`BITMAPV5HEADER` fields this needs; both share the same first 40
    /// bytes. A negative height means the rows are stored top-down instead of bottom-up.
    struct DibHeader {
        width: u32,
        height: u32,
        top_down: bool,
        bit_count: u16,
        compression: u32,
    }

    fn dib_header(head: &[u8]) -> Result<DibHeader, String> {
        if head.len() < 40 {
            return Err(BAD_IMAGE.into());
        }
        let width = i32::from_le_bytes(head[4..8].try_into().unwrap());
        let height = i32::from_le_bytes(head[8..12].try_into().unwrap());
        let bit_count = u16::from_le_bytes(head[14..16].try_into().unwrap());
        let compression = u32::from_le_bytes(head[16..20].try_into().unwrap());
        if width <= 0 || height == 0 {
            return Err(BAD_IMAGE.into());
        }
        Ok(DibHeader { width: width as u32, height: height.unsigned_abs(), top_down: height < 0, bit_count, compression })
    }

    /// `BI_BITFIELDS` colour masks: embedded fields on a V4/V5 header, or three trailing `DWORD`s
    /// right after an old-style 40-byte `BITMAPINFOHEADER` (which never carries an alpha mask).
    fn bitfield_masks(dib: &[u8], bi_size: usize) -> Option<(u32, u32, u32, u32)> {
        let read = |offset: usize| -> Option<u32> { Some(u32::from_le_bytes(dib.get(offset..offset + 4)?.try_into().ok()?)) };
        if bi_size >= 56 {
            Some((read(40)?, read(44)?, read(48)?, read(52)?))
        } else {
            Some((read(40)?, read(44)?, read(48)?, 0))
        }
    }

    /// Pulls an 8-bit channel out of a packed pixel using its mask; a zero mask (only ever the
    /// alpha one) means the channel does not exist, so `default` (255, fully opaque) is used.
    fn channel(pixel: u32, mask: u32, default: u8) -> u8 {
        if mask == 0 {
            return default;
        }
        let shift = mask.trailing_zeros();
        let width = 32 - mask.leading_zeros() - shift;
        let value = (pixel & mask) >> shift;
        if width >= 8 { (value >> (width - 8)) as u8 } else { ((value * 255) / ((1u32 << width) - 1).max(1)) as u8 }
    }

    /// `dib` is the full clipboard block (header, any colour masks, then pixel rows); only
    /// 24bpp/32bpp `BI_RGB` and 32bpp `BI_BITFIELDS` are handled, which covers Paint, Word and
    /// every other real sender of plain DIBs — anything else is refused with a clear message.
    fn dib_to_png(header: &DibHeader, dib: &[u8]) -> Result<Vec<u8>, String> {
        let bi_size = u32::from_le_bytes(dib.get(0..4).and_then(|s| s.try_into().ok()).ok_or(BAD_IMAGE)?) as usize;
        let width = header.width as usize;
        let height = header.height as usize;
        let bytes_per_pixel = match header.bit_count {
            24 => 3,
            32 => 4,
            _ => return Err(UNSUPPORTED_DIB.into()),
        };
        let masks = if header.compression == BI_BITFIELDS.0 {
            if header.bit_count != 32 {
                return Err(UNSUPPORTED_DIB.into());
            }
            Some(bitfield_masks(dib, bi_size).ok_or(BAD_IMAGE)?)
        } else if header.compression == BI_RGB.0 {
            None
        } else {
            return Err(UNSUPPORTED_DIB.into());
        };
        let data_offset = if masks.is_some() && bi_size == 40 { bi_size + 12 } else { bi_size };
        // Rows are padded to a 4-byte boundary; only 24bpp rows are ever not already aligned.
        let row_len = (width * bytes_per_pixel).div_ceil(4) * 4;
        let pixels = dib.get(data_offset..).ok_or(BAD_IMAGE)?;
        if pixels.len() < row_len * height {
            return Err(BAD_IMAGE.into());
        }
        let mut rgba = vec![0u8; width * height * 4];
        for y in 0..height {
            let source_row = if header.top_down { y } else { height - 1 - y };
            let row = &pixels[source_row * row_len..source_row * row_len + width * bytes_per_pixel];
            for x in 0..width {
                let pixel = &row[x * bytes_per_pixel..x * bytes_per_pixel + bytes_per_pixel];
                let out = &mut rgba[(y * width + x) * 4..(y * width + x) * 4 + 4];
                match masks {
                    None => out.copy_from_slice(&[pixel[2], pixel[1], pixel[0], 255]),
                    Some((red, green, blue, alpha)) => {
                        let packed = u32::from_le_bytes([pixel[0], pixel[1], pixel[2], pixel[3]]);
                        out.copy_from_slice(&[channel(packed, red, 0), channel(packed, green, 0), channel(packed, blue, 0), channel(packed, alpha, 255)]);
                    }
                }
            }
        }
        let mut png = Vec::new();
        PngEncoder::new(&mut png)
            .write_image(&rgba, header.width, header.height, ExtendedColorType::Rgba8)
            .map_err(|error| error.to_string())?;
        Ok(png)
    }

    /// Runs inside `open_with`'s `spawn_blocking`, never on the main thread. Never decodes a
    /// full image before checking its declared size: the clipboard is untrusted input, since any
    /// app can have put something oversized there (OWASP A10:2025, exceptional-condition DoS).
    pub fn pasteboard_image() -> Result<Vec<u8>, String> {
        let _clipboard = ClipboardGuard::open()?;
        if let Some(handle) = clipboard_data(png_format()) {
            let locked = GlobalBytes::lock(handle)?;
            let (width, height) = png_ihdr_size(locked.head(24)?)?;
            super::check_size(width, height)?;
            locked.check_len(super::MAX_FILE_BYTES)?;
            return Ok(locked.to_vec());
        }
        if let Some(handle) = clipboard_data(u32::from(CF_DIBV5.0)).or_else(|| clipboard_data(u32::from(CF_DIB.0))) {
            let locked = GlobalBytes::lock(handle)?;
            let header = dib_header(locked.head(40)?)?;
            super::check_size(header.width, header.height)?;
            locked.check_len(super::MAX_FILE_BYTES)?;
            let dib = locked.to_vec();
            drop(locked);
            return dib_to_png(&header, &dib);
        }
        Err("El portapapeles no tiene ninguna imagen".into())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn sample_png_head() -> Vec<u8> {
            use xcap::image::{Rgba, RgbaImage};
            let path = std::env::temp_dir().join(format!("snapclone-clipboard-{}.png", uuid::Uuid::new_v4()));
            crate::images::write_png(&path, &RgbaImage::from_pixel(5, 3, Rgba([10, 20, 30, 255]))).unwrap();
            let bytes = std::fs::read(&path).unwrap();
            std::fs::remove_file(path).unwrap();
            bytes
        }

        #[test]
        fn png_ihdr_size_reads_a_real_header() {
            let png = sample_png_head();
            assert_eq!(png_ihdr_size(&png[..24]), Ok((5, 3)));
        }

        #[test]
        fn png_ihdr_size_rejects_truncated_or_foreign_bytes() {
            let png = sample_png_head();
            assert!(png_ihdr_size(&png[..10]).is_err());
            assert!(png_ihdr_size(b"not a png at all, just text").is_err());
        }

        fn bmp_header(width: i32, height: i32, bit_count: u16, compression: u32) -> Vec<u8> {
            let mut head = vec![0u8; 40];
            head[0..4].copy_from_slice(&40u32.to_le_bytes());
            head[4..8].copy_from_slice(&width.to_le_bytes());
            head[8..12].copy_from_slice(&height.to_le_bytes());
            head[14..16].copy_from_slice(&bit_count.to_le_bytes());
            head[16..20].copy_from_slice(&compression.to_le_bytes());
            head
        }

        #[test]
        fn dib_header_reads_width_height_and_bottom_up_by_default() {
            let header = dib_header(&bmp_header(4, 3, 24, BI_RGB.0)).unwrap();
            assert_eq!((header.width, header.height, header.top_down), (4, 3, false));
        }

        #[test]
        fn dib_header_treats_a_negative_height_as_top_down() {
            let header = dib_header(&bmp_header(4, -3, 24, BI_RGB.0)).unwrap();
            assert_eq!((header.width, header.height, header.top_down), (4, 3, true));
        }

        #[test]
        fn dib_header_rejects_a_truncated_or_empty_header() {
            assert!(dib_header(&bmp_header(4, 3, 24, BI_RGB.0)[..30]).is_err());
            assert!(dib_header(&bmp_header(0, 3, 24, BI_RGB.0)).is_err());
            assert!(dib_header(&bmp_header(4, 0, 24, BI_RGB.0)).is_err());
        }

        /// A 2×2 24bpp BGR DIB, bottom-up, with the 4-byte row padding every real DIB has.
        fn dib_24bpp(pixels: [[u8; 3]; 4]) -> Vec<u8> {
            let mut dib = bmp_header(2, 2, 24, BI_RGB.0);
            for row in [[pixels[2], pixels[3]], [pixels[0], pixels[1]]] {
                for [b, g, r] in row {
                    dib.extend_from_slice(&[b, g, r]);
                }
                dib.extend_from_slice(&[0, 0]); // pad 6 bytes up to the next multiple of 4
            }
            dib
        }

        #[test]
        fn dib_to_png_decodes_24bpp_bgr_bottom_up() {
            let red = [0, 0, 255];
            let green = [0, 255, 0];
            let blue = [255, 0, 0];
            let white = [255, 255, 255];
            let dib = dib_24bpp([red, green, blue, white]);
            let header = dib_header(&dib).unwrap();
            let png = dib_to_png(&header, &dib).unwrap();
            let image = xcap::image::load_from_memory(&png).unwrap().to_rgba8();
            assert_eq!(image.get_pixel(0, 0).0, [255, 0, 0, 255]);
            assert_eq!(image.get_pixel(1, 0).0, [0, 255, 0, 255]);
            assert_eq!(image.get_pixel(0, 1).0, [0, 0, 255, 255]);
            assert_eq!(image.get_pixel(1, 1).0, [255, 255, 255, 255]);
        }

        #[test]
        fn dib_to_png_decodes_32bpp_bitfields_with_alpha() {
            // A V4/V5-shaped header (`biSize` >= 56): the colour masks are embedded fields, not
            // a trailing table, so the pixel data starts right after the header, not 12 bytes in.
            let mut dib = bmp_header(2, 1, 32, BI_BITFIELDS.0);
            dib[0..4].copy_from_slice(&108u32.to_le_bytes());
            dib.resize(108, 0);
            dib[40..44].copy_from_slice(&0x00FF_0000u32.to_le_bytes()); // red mask
            dib[44..48].copy_from_slice(&0x0000_FF00u32.to_le_bytes()); // green mask
            dib[48..52].copy_from_slice(&0x0000_00FFu32.to_le_bytes()); // blue mask
            dib[52..56].copy_from_slice(&0xFF00_0000u32.to_le_bytes()); // alpha mask
            dib.extend_from_slice(&0x80FF_0000u32.to_le_bytes()); // pixel (0,0): opaque-ish red
            dib.extend_from_slice(&0x0000_FF00u32.to_le_bytes()); // pixel (1,0): transparent green
            let header = dib_header(&dib).unwrap();
            let png = dib_to_png(&header, &dib).unwrap();
            let image = xcap::image::load_from_memory(&png).unwrap().to_rgba8();
            assert_eq!(image.get_pixel(0, 0).0, [255, 0, 0, 128]);
            assert_eq!(image.get_pixel(1, 0).0, [0, 255, 0, 0]);
        }

        #[test]
        fn dib_to_png_rejects_unsupported_bit_depths_and_compression() {
            let eight_bpp = dib_header(&bmp_header(2, 2, 8, BI_RGB.0)).unwrap();
            assert!(dib_to_png(&eight_bpp, &bmp_header(2, 2, 8, BI_RGB.0)).is_err());
            let rle = dib_header(&bmp_header(2, 2, 24, 2)).unwrap(); // an RLE compression value
            assert!(dib_to_png(&rle, &bmp_header(2, 2, 24, 2)).is_err());
        }

        #[test]
        fn a_header_declaring_an_enormous_image_is_rejected_before_any_large_allocation() {
            let huge = dib_header(&bmp_header(30_000, 30_000, 24, BI_RGB.0)).unwrap();
            assert!(super::super::check_size(huge.width, huge.height).is_err());
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod native {
    /// Without ImageIO the PNG header check after conversion is the only one.
    pub fn header_size(_bytes: &[u8]) -> Option<(u32, u32)> {
        Some((1, 1))
    }

    pub fn pasteboard_image() -> Result<Vec<u8>, String> {
        Err("El portapapeles no tiene ninguna imagen".into())
    }

    pub fn to_png(_bytes: &[u8]) -> Result<Vec<u8>, String> {
        Err("converting images is only implemented on macOS".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xcap::image::{Rgba, RgbaImage};

    fn sample_png(width: u32, height: u32) -> Vec<u8> {
        let path = std::env::temp_dir().join(format!("snapclone-import-{}.png", uuid::Uuid::new_v4()));
        crate::images::write_png(&path, &RgbaImage::from_pixel(width, height, Rgba([200, 40, 90, 255]))).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        std::fs::remove_file(path).unwrap();
        bytes
    }

    const ALL: [Format; 3] = [Format::Png, Format::Jpeg, Format::Tiff];

    #[test]
    fn formats_are_recognised_by_their_bytes() {
        assert_eq!(sniff(&sample_png(2, 2)), Some(Format::Png));
        assert_eq!(sniff(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 0]), Some(Format::Jpeg));
        assert_eq!(sniff(b"II*\0rest"), Some(Format::Tiff));
        assert_eq!(sniff(b"MM\0*rest"), Some(Format::Tiff));
        assert_eq!(sniff(b"GIF89a"), None);
        assert_eq!(sniff(b""), None);
    }

    #[test]
    fn pngs_are_kept_byte_for_byte_at_scale_one() {
        let png = sample_png(40, 30);
        let (stored, scale) = prepare(png.clone(), &ALL, "no").unwrap();
        assert_eq!(stored, png);
        assert_eq!(scale, 1.0);
    }

    #[test]
    fn formats_outside_the_accepted_list_are_rejected() {
        let error = prepare(sample_png(4, 4), &[Format::Jpeg], "Solo JPEG").unwrap_err();
        assert_eq!(error, "Solo JPEG");
        assert_eq!(prepare(b"GIF89a....".to_vec(), &ALL, "Formato no admitido").unwrap_err(), "Formato no admitido");
    }

    #[test]
    fn truncated_png_is_reported_not_stored() {
        let mut png = sample_png(10, 10);
        png.truncate(12);
        assert!(prepare(png, &ALL, "no").is_err());
    }

    #[test]
    fn files_are_read_only_when_they_are_regular_and_small() {
        let dir = std::env::temp_dir().join(format!("snapclone-import-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(read_file(&dir).unwrap_err(), FILE_UNSUPPORTED, "a folder is not an image");
        assert!(read_file(&dir.join("missing.png")).is_err());
        let file = dir.join("captura.png");
        std::fs::write(&file, sample_png(3, 3)).unwrap();
        let (png, _) = prepare(read_file(&file).unwrap(), FILE_FORMATS, FILE_UNSUPPORTED).unwrap();
        assert_eq!(sniff(&png), Some(Format::Png));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn files_must_be_png_or_jpeg_whatever_their_name() {
        let tiff_bytes = b"II*\0 pretending to be a png".to_vec();
        assert_eq!(prepare(tiff_bytes, FILE_FORMATS, FILE_UNSUPPORTED).unwrap_err(), FILE_UNSUPPORTED);
    }

    /// A real PNG far wider than the editor allows; 1 px tall, so it stays tiny.
    fn wide_png(width: u32) -> Vec<u8> {
        use xcap::image::{ExtendedColorType, ImageEncoder, codecs::png::PngEncoder};
        let mut png = Vec::new();
        PngEncoder::new(&mut png).write_image(&vec![0; width as usize], width, 1, ExtendedColorType::L8).unwrap();
        png
    }

    #[test]
    fn oversized_images_are_refused_from_their_header() {
        let png = wide_png(40_000);
        assert_eq!(native::header_size(&png), Some((40_000, 1)), "read from the header");
        let error = prepare(png, &ALL, "no").unwrap_err();
        assert!(error.contains("demasiado grande"), "{error}");
    }

    #[test]
    fn decode_limits_cover_the_largest_image_the_app_allows() {
        let limits = decode_limits();
        assert!(limits.max_alloc.unwrap() >= MAX_PIXELS * 4);
        assert_eq!(limits.max_image_width, Some(MAX_SIDE));
        assert_eq!(limits.max_image_height, Some(MAX_SIDE));
    }

    /// A real JPEG just as wide as `wide_png`'s PNG, over the JPEG decoder this phase adds for
    /// Windows (not available on macOS, which never enables the `jpeg` feature of `image`).
    #[cfg(windows)]
    fn wide_jpeg(width: u32) -> Vec<u8> {
        use xcap::image::{ExtendedColorType, codecs::jpeg::JpegEncoder};
        let mut jpeg = Vec::new();
        JpegEncoder::new(&mut jpeg).encode(&vec![0u8; width as usize], width, 1, ExtendedColorType::L8).unwrap();
        jpeg
    }

    #[cfg(windows)]
    #[test]
    fn an_oversized_jpeg_is_refused_from_its_header_too() {
        let jpeg = wide_jpeg(40_000);
        assert_eq!(native::header_size(&jpeg), Some((40_000, 1)), "read from the header, not a full decode");
        let error = prepare(jpeg, &ALL, "no").unwrap_err();
        assert!(error.contains("demasiado grande"), "{error}");
    }

    #[test]
    fn sizes_beyond_what_the_editor_can_draw_are_rejected() {
        assert!(check_size(1920, 1080).is_ok());
        assert!(check_size(1000, 30_000).is_ok());
        assert!(check_size(0, 10).is_err());
        assert!(check_size(40_000, 10).is_err());
        assert!(check_size(20_000, 20_000).is_err());
    }

    /// macOS converts JPEG and TIFF (ImageIO, unchanged); Windows only JPEG, since no path in
    /// the app ever needs a TIFF decoder there (`FILE_FORMATS` never accepts `Format::Tiff`).
    #[cfg(target_os = "macos")]
    fn encoded_samples(png: &[u8]) -> Vec<Vec<u8>> {
        use objc2_app_kit::NSBitmapImageFileType;
        vec![native::encode(png, NSBitmapImageFileType::JPEG), native::encode(png, NSBitmapImageFileType::TIFF)]
    }

    #[cfg(windows)]
    fn encoded_samples(png: &[u8]) -> Vec<Vec<u8>> {
        vec![native::encode(png, xcap::image::ImageFormat::Jpeg)]
    }

    #[cfg(not(any(target_os = "macos", windows)))]
    fn encoded_samples(_png: &[u8]) -> Vec<Vec<u8>> {
        Vec::new()
    }

    #[test]
    fn jpeg_and_tiff_are_converted_to_png_with_their_size() {
        let png = sample_png(64, 48);
        for encoded in encoded_samples(&png) {
            let (converted, scale) = prepare(encoded, &ALL, "no").unwrap();
            assert_eq!(sniff(&converted), Some(Format::Png));
            let image = xcap::image::load_from_memory(&converted).unwrap();
            assert_eq!((image.width(), image.height()), (64, 48));
            assert_eq!(scale, 1.0);
        }
    }
}
