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
const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;

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
/// they are, so their colour profile and `pHYs` survive; other formats are re-encoded.
pub fn prepare(bytes: Vec<u8>, accepted: &[Format], unsupported: &str) -> Result<(Vec<u8>, f64), String> {
    let format = sniff(&bytes).filter(|format| accepted.contains(format)).ok_or(unsupported)?;
    let png = match format {
        Format::Png => bytes,
        Format::Jpeg | Format::Tiff => native::to_png(&bytes)?,
    };
    let (width, height) = ImageReader::new(Cursor::new(&png))
        .with_guessed_format()
        .map_err(|error| error.to_string())?
        .into_dimensions()
        .map_err(|_| "La imagen está dañada o no se puede leer".to_string())?;
    check_size(width, height)?;
    let scale = capture::png_scale(&png).min(MAX_SCALE);
    Ok((png, scale))
}

fn check_size(width: u32, height: u32) -> Result<(), String> {
    if width == 0 || height == 0 {
        return Err("La imagen está vacía".into());
    }
    if width > MAX_SIDE || height > MAX_SIDE || u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err(format!("La imagen es demasiado grande para editarla ({width} × {height} px)"));
    }
    Ok(())
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
    if let Some(path) = paths.first() {
        open_path(app, path.clone());
    }
}

#[tauri::command]
pub fn open_image_file(app: AppHandle) {
    open_file_dialog(&app);
}

/// Opens the image on the pasteboard. AppKit is read here, so call it on the main thread
/// (menu events and synchronous commands run there).
pub fn open_clipboard(app: &AppHandle) {
    match native::pasteboard_image() {
        Some(bytes) => open_with(app, move || Ok(bytes), &[Format::Png, Format::Tiff], "El portapapeles no tiene una imagen PNG o TIFF"),
        None => editor_window::notify(app, "El portapapeles no tiene ninguna imagen", true),
    }
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

#[cfg(not(target_os = "macos"))]
mod native {
    pub fn pasteboard_image() -> Option<Vec<u8>> {
        None
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

    #[test]
    fn sizes_beyond_what_the_editor_can_draw_are_rejected() {
        assert!(check_size(1920, 1080).is_ok());
        assert!(check_size(1000, 30_000).is_ok());
        assert!(check_size(0, 10).is_err());
        assert!(check_size(40_000, 10).is_err());
        assert!(check_size(20_000, 20_000).is_err());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn jpeg_and_tiff_are_converted_to_png_with_their_size() {
        use objc2_app_kit::NSBitmapImageFileType;
        let png = sample_png(64, 48);
        for kind in [NSBitmapImageFileType::JPEG, NSBitmapImageFileType::TIFF] {
            let encoded = native::encode(&png, kind);
            let (converted, scale) = prepare(encoded, &ALL, "no").unwrap();
            assert_eq!(sniff(&converted), Some(Format::Png));
            let image = xcap::image::load_from_memory(&converted).unwrap();
            assert_eq!((image.width(), image.height()), (64, 48));
            assert_eq!(scale, 1.0);
        }
    }
}
