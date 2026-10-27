use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
};

use serde::Serialize;
use tauri::{http, AppHandle, Emitter, Manager};

use crate::{editor_window, history, images, selector};

/// Lets the editor window finish hiding so it is not in the frozen screenshots.
const HIDE_SETTLE: std::time::Duration = std::time::Duration::from_millis(200);

pub const EDITOR_WINDOW: &str = "main";
const READY_EVENT: &str = "capture-ready";
const FAILED_EVENT: &str = "capture-failed";
/// `capture://localhost/thumbnail/<id>` serves the small preview of a capture.
const THUMBNAIL_PREFIX: &str = "thumbnail/";

/// Captured files by id, served to the editor through the `capture://` scheme.
#[derive(Default)]
pub struct Captures {
    files: Mutex<HashMap<String, PathBuf>>,
    in_progress: AtomicBool,
}

#[derive(Clone, Serialize)]
struct CaptureReady {
    id: String,
    /// Pixels per point of the captured display, so styling lengths match the image.
    scale: f64,
}

/// Path of a capture taken in this session or stored in the history, if the id is known.
/// Paths only ever come from Rust, never from the webview.
pub fn path_of(app: &AppHandle, id: &str) -> Option<PathBuf> {
    let session = app.state::<Captures>().files.lock().unwrap().get(id).cloned();
    session.or_else(|| history::path_of(app, id))
}

/// Drops a deleted capture from this session's lookup table.
pub fn forget(app: &AppHandle, id: &str) {
    app.state::<Captures>().files.lock().unwrap().remove(id);
}

/// Starts a region capture in the background; ignored while another one is running.
pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(capture_region(app));
}

#[tauri::command]
pub fn start_capture(app: AppHandle) {
    start(app);
}

async fn capture_region(app: AppHandle) {
    let captures = app.state::<Captures>();
    if captures.in_progress.swap(true, Ordering::SeqCst) {
        return;
    }

    if let Some(window) = app.get_webview_window(EDITOR_WINDOW)
        && window.is_visible().unwrap_or(false)
    {
        let _ = window.hide();
        tokio::time::sleep(HIDE_SETTLE).await;
    }

    // The own selector keeps `in_progress` set until the selection ends; see `selection_ended`.
    let opened = tauri::async_runtime::spawn_blocking({
        let app = app.clone();
        move || selector::open(&app)
    })
    .await
    .unwrap_or_else(|error| Err(error.to_string()));
    match opened {
        Ok(()) => return,
        Err(error) => eprintln!("own selector unavailable, using screencapture -i: {error}"),
    }

    let result = match capture_dir(&app) {
        Ok(dir) => run_screencapture(&dir).await,
        Err(error) => Err(error),
    };
    match result {
        Ok(Some((id, path))) => {
            let scale = std::fs::read(&path).map_or(1.0, |bytes| png_scale(&bytes));
            deliver(&app, id, path, scale);
        }
        Ok(None) => {}
        Err(error) => report_failure(&app, &error),
    }
    captures.in_progress.store(false, Ordering::SeqCst);
}

/// Brings the editor back with the error in its status line, so a failed capture is never
/// lost silently behind a hidden window.
pub fn report_failure(app: &AppHandle, error: &str) {
    eprintln!("capture failed: {error}");
    editor_window::show(app, false);
    if let Err(emit_error) = app.emit_to(EDITOR_WINDOW, FAILED_EVENT, error) {
        eprintln!("capture-failed emit failed: {emit_error}");
    }
}

/// Called when the own selector closes, whether a capture was taken or not.
pub fn selection_ended(app: &AppHandle) {
    app.state::<Captures>().in_progress.store(false, Ordering::SeqCst);
}

/// Registers a finished capture, stores it in the history and hands it to the editor.
pub fn deliver(app: &AppHandle, id: String, path: PathBuf, scale: f64) {
    history::record(app, &id, &path, scale);
    // The history view only needs it later, so it never delays showing the capture.
    let source = path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if let Err(error) = images::write_thumbnail(&source) {
            eprintln!("thumbnail could not be written: {error}");
        }
    });
    app.state::<Captures>().files.lock().unwrap().insert(id.clone(), path);
    editor_window::show(app, false);
    if let Err(error) = app.emit_to(EDITOR_WINDOW, READY_EVENT, CaptureReady { id, scale }) {
        eprintln!("capture-ready emit failed: {error}");
    }
}

/// Captures live in the app data directory, next to the history that points at them, so
/// the system never purges them the way it may purge caches.
pub fn capture_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?
        .join("captures");
    std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    Ok(dir)
}

/// Runs the native interactive selector, the fallback when the own one cannot capture the
/// screens. Returns `None` when the user cancels with Esc.
async fn run_screencapture(dir: &Path) -> Result<Option<(String, PathBuf)>, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let path = dir.join(format!("{id}.png"));
    let status = tokio::process::Command::new("/usr/sbin/screencapture")
        // -o drops the macOS window shadow; the editor draws its own.
        .args(["-i", "-x", "-o"])
        .arg(&path)
        .status()
        .await
        .map_err(|error| error.to_string())?;
    // Cancelling leaves no file behind, so its absence is the reliable signal.
    if !status.success() || !path.exists() {
        return Ok(None);
    }
    Ok(Some((id, path)))
}

/// Reads the display scale from the PNG `pHYs` chunk: screencapture stores 144 DPI for
/// Retina captures and omits the chunk at 1x, which falls back to 1.
fn png_scale(png: &[u8]) -> f64 {
    const POINTS_PER_INCH: f64 = 72.0;
    const METERS_PER_INCH: f64 = 0.0254;
    let mut offset = 8;
    while let Some(header) = png.get(offset..offset + 8) {
        let length = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
        let kind = &header[4..8];
        if kind == b"IDAT" {
            break;
        }
        if kind == b"pHYs"
            && let Some(data) = png.get(offset + 8..offset + 17)
            && data[8] == 1
        {
            let pixels_per_meter = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
            let scale = (f64::from(pixels_per_meter) * METERS_PER_INCH / POINTS_PER_INCH).round();
            return scale.max(1.0);
        }
        offset += 12 + length;
    }
    1.0
}

/// Serves `capture://localhost/<id>` with CORS so the editor canvas stays exportable.
pub fn serve(app: &AppHandle, request: &http::Request<Vec<u8>>) -> http::Response<Vec<u8>> {
    let requested = request.uri().path().trim_start_matches('/');
    let path = match requested.strip_prefix(THUMBNAIL_PREFIX) {
        Some(id) => path_of(app, id).map(|path| thumbnail_or_capture(&path)),
        None => path_of(app, requested),
    };
    let response = http::Response::builder().header("Access-Control-Allow-Origin", "*");
    match path.and_then(|path| std::fs::read(path).ok()) {
        Some(bytes) => response.header("Content-Type", "image/png").body(bytes),
        None => response.status(404).body(Vec::new()),
    }
    .expect("static response parts are valid")
}

/// Captures from before thumbnails existed get one on first request; if that fails the
/// full image is served so the history still shows something.
fn thumbnail_or_capture(capture: &Path) -> PathBuf {
    let thumbnail = images::thumbnail_path(capture);
    if thumbnail.exists() || images::write_thumbnail(capture).is_ok() {
        thumbnail
    } else {
        capture.to_path_buf()
    }
}

#[cfg(test)]
mod tests {
    use super::png_scale;

    fn png_with_chunks(chunks: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        for (kind, data) in chunks {
            png.extend((data.len() as u32).to_be_bytes());
            png.extend(*kind);
            png.extend(data);
            png.extend([0; 4]); // CRC is not checked
        }
        png
    }

    fn phys(pixels_per_meter: u32, unit: u8) -> Vec<u8> {
        let mut data = pixels_per_meter.to_be_bytes().to_vec();
        data.extend(pixels_per_meter.to_be_bytes());
        data.push(unit);
        data
    }

    #[test]
    fn retina_capture_is_scale_two() {
        let png = png_with_chunks(&[(b"IHDR", vec![0; 13]), (b"pHYs", phys(5669, 1)), (b"IDAT", vec![])]);
        assert_eq!(png_scale(&png), 2.0);
    }

    #[test]
    fn missing_or_unitless_phys_is_scale_one() {
        assert_eq!(png_scale(&png_with_chunks(&[(b"IHDR", vec![0; 13]), (b"IDAT", vec![])])), 1.0);
        assert_eq!(png_scale(&png_with_chunks(&[(b"pHYs", phys(5669, 0))])), 1.0);
        assert_eq!(png_scale(b"not a png"), 1.0);
    }
}
