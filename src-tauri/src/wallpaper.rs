//! The desktop picture of the main display, which the editor blurs into a background like
//! Xnapper does. Its path comes from AppKit; the webview only ever asks for
//! `capture://localhost/wallpaper` and can never name a file.

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use tauri::AppHandle;

/// AppKit is asked on the main thread; this bounds the wait if that thread is busy.
const MAIN_THREAD_TIMEOUT: Duration = Duration::from_secs(2);

/// Formats WebKit decodes, by file extension, with the type they are served as.
const CONTENT_TYPES: [(&str, &str); 7] = [
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("heic", "image/heic"),
    ("heif", "image/heif"),
    ("tif", "image/tiff"),
    ("tiff", "image/tiff"),
];

/// Type to serve a wallpaper file as; `None` for folders (rotating or dynamic wallpapers)
/// and formats the editor cannot draw.
pub fn content_type(path: &Path) -> Option<&'static str> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    CONTENT_TYPES.iter().find(|(known, _)| *known == extension).map(|(_, kind)| *kind)
}

/// Reads the current wallpaper with the type to serve it as. Errors are user-facing.
pub fn read(app: &AppHandle) -> Result<(Vec<u8>, &'static str), String> {
    let path = current_path(app)?;
    let kind = content_type(&path).ok_or("macOS no da el fondo de escritorio como una imagen (¿es dinámico o rotativo?)")?;
    let bytes = std::fs::read(&path).map_err(|error| format!("no se pudo leer el fondo de escritorio: {error}"))?;
    Ok((bytes, kind))
}

/// AppKit's screen list only exists on the main thread, while `capture://` is served off it.
fn current_path(app: &AppHandle) -> Result<PathBuf, String> {
    let (sender, receiver) = std::sync::mpsc::channel();
    app.run_on_main_thread(move || {
        let _ = sender.send(desktop_image_path());
    })
    .map_err(|error| error.to_string())?;
    receiver.recv_timeout(MAIN_THREAD_TIMEOUT).map_err(|_| "macOS no respondió a tiempo".to_string())?
}

/// The first screen is the one with the menu bar, which macOS calls the main display.
#[cfg(target_os = "macos")]
fn desktop_image_path() -> Result<PathBuf, String> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSScreen, NSWorkspace};

    let main_thread = MainThreadMarker::new().ok_or("not on the main thread")?;
    let screen = NSScreen::screens(main_thread).firstObject().ok_or("no hay ninguna pantalla")?;
    let url = NSWorkspace::sharedWorkspace()
        .desktopImageURLForScreen(&screen)
        .ok_or("macOS no da el fondo de escritorio")?;
    let path = url.path().ok_or("el fondo de escritorio no es un fichero")?;
    Ok(PathBuf::from(path.to_string()))
}

#[cfg(not(target_os = "macos"))]
fn desktop_image_path() -> Result<PathBuf, String> {
    Err("the desktop picture is only read on macOS".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serves_decodable_images_by_extension() {
        assert_eq!(content_type(Path::new("/System/Library/Desktop Pictures/Sonoma.heic")), Some("image/heic"));
        assert_eq!(content_type(Path::new("/Users/x/Fondo.JPG")), Some("image/jpeg"));
        assert_eq!(content_type(Path::new("/Users/x/fondo.png")), Some("image/png"));
    }

    #[test]
    fn folders_and_unknown_formats_are_not_served() {
        assert_eq!(content_type(Path::new("/System/Library/Desktop Pictures/.thumbnails")), None);
        assert_eq!(content_type(Path::new("/Users/x/Fotos")), None);
        assert_eq!(content_type(Path::new("/Users/x/fondo.madesktop")), None);
    }
}
