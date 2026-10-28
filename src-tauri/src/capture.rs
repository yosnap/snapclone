use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
};

use serde::{Deserialize, Serialize};
use tauri::{http, AppHandle, Emitter, Manager};
use xcap::image::RgbaImage;

use crate::{
    app_settings::{self, Toggle},
    editor_window, history, hud, images, screen, selector, wallpaper,
};

/// Lets the editor window finish hiding so it is not in the frozen screenshots.
const HIDE_SETTLE: std::time::Duration = std::time::Duration::from_millis(200);
/// How long the hidden editor has to style and copy a capture before it is shown instead.
/// Generous, since detecting sensitive text in a large capture takes a few seconds.
const COPY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

pub const EDITOR_WINDOW: &str = "main";
const READY_EVENT: &str = "capture-ready";
const FAILED_EVENT: &str = "capture-failed";
/// `capture://localhost/thumbnail/<id>` serves the small preview of a capture.
const THUMBNAIL_PREFIX: &str = "thumbnail/";
/// Sent with every `capture://localhost/wallpaper…` response, so the editor can tell when
/// the desktop picture changed. Capture ids are UUIDs, so they never clash with those paths.
const WALLPAPER_VERSION_HEADER: &str = "X-Wallpaper-Version";

/// Captured files by id, served to the editor through the `capture://` scheme.
#[derive(Default)]
pub struct Captures {
    files: Mutex<HashMap<String, PathBuf>>,
    in_progress: AtomicBool,
    /// The windows hidden for the capture in progress, shown again when it ends.
    hidden: Mutex<Option<HiddenWindows>>,
    /// Whether the capture in progress is copied instead of opened in the editor. Set only
    /// by the capture that owns `in_progress`, and cleared when it ends.
    copying: AtomicBool,
    /// The capture the hidden editor is copying, until it reports back or runs out of time.
    pending_copy: Mutex<Option<String>>,
}

/// Which of Snapclone's windows were on screen when a capture hid them.
#[derive(Clone, Copy)]
struct HiddenWindows {
    editor: bool,
    settings: bool,
}

#[derive(Clone, Serialize)]
struct CaptureReady {
    id: String,
    /// Pixels per point of the captured display, so styling lengths match the image.
    scale: f64,
    /// Style and copy it without showing the editor, then call `copy_finished`.
    copy: bool,
}

/// What a capture takes; the selector and the fallback tool work per kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CaptureKind {
    /// A dragged zone in the own selector.
    Region,
    /// The selector, starting in window mode.
    Window,
    /// The monitor under the pointer, without any selector.
    Screen,
    /// A zone captured repeatedly while the user scrolls, stitched into one tall image.
    Scroll,
}

/// What a capture shortcut or menu entry does: a capture kind, and whether the result opens
/// in the editor or is styled and copied straight to the clipboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CaptureAction {
    Region,
    /// A zone styled like the editor would and copied, without showing the editor.
    RegionCopy,
    Window,
    Screen,
    Scroll,
}

impl CaptureAction {
    /// In the order the settings and the menu list them.
    pub const ALL: [Self; 5] = [Self::Region, Self::RegionCopy, Self::Window, Self::Screen, Self::Scroll];

    /// Name in the settings file and in menu ids; the same as its serialized form.
    pub fn id(self) -> &'static str {
        match self {
            Self::Region => "region",
            Self::RegionCopy => "region-copy",
            Self::Window => "window",
            Self::Screen => "screen",
            Self::Scroll => "scroll",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|action| action.id() == id)
    }

    /// As the user reads it in the settings and in messages.
    pub fn label(self) -> &'static str {
        match self {
            Self::Region => "Capturar y abrir el editor",
            Self::RegionCopy => "Capturar y copiar al portapapeles",
            Self::Window => "Capturar ventana",
            Self::Screen => "Capturar pantalla completa",
            Self::Scroll => "Capturar con scroll",
        }
    }

    /// The menu bar entry, which names the zone the two zone actions take.
    pub fn menu_title(self) -> &'static str {
        match self {
            Self::Region => "Capturar zona",
            Self::RegionCopy => "Capturar zona y copiar",
            other => other.label(),
        }
    }

    pub fn kind(self) -> CaptureKind {
        match self {
            Self::Region | Self::RegionCopy => CaptureKind::Region,
            Self::Window => CaptureKind::Window,
            Self::Screen => CaptureKind::Screen,
            Self::Scroll => CaptureKind::Scroll,
        }
    }

    fn copies(self) -> bool {
        self == Self::RegionCopy
    }
}

impl CaptureKind {
    /// Arguments for `screencapture` when the own capture is unavailable.
    fn fallback_args(self) -> &'static [&'static str] {
        // -x: no sound; -o: no window shadow, the editor draws its own.
        match self {
            Self::Window => &["-i", "-W", "-x", "-o"],
            Self::Screen => &["-x", "-o"],
            Self::Region | Self::Scroll => &["-i", "-x", "-o"],
        }
    }
}

/// Without Screen Recording permission CoreGraphics still "succeeds" but returns only the
/// wallpaper, so the permission is checked up front. When it is missing, macOS is asked to
/// prompt for it and the caller falls back to `screencapture`, which handles its own prompt.
pub fn screen_capture_allowed() -> bool {
    if has_screen_capture_permission() {
        return true;
    }
    request_screen_capture_permission();
    false
}

#[cfg(target_os = "macos")]
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
}

/// Only asks whether the permission is granted; never prompts.
#[cfg(target_os = "macos")]
fn has_screen_capture_permission() -> bool {
    // SAFETY: takes no arguments and only queries the permission.
    unsafe { CGPreflightScreenCaptureAccess() }
}

#[cfg(target_os = "macos")]
fn request_screen_capture_permission() {
    // SAFETY: takes no arguments and only shows the system prompt.
    unsafe {
        CGRequestScreenCaptureAccess();
    }
}

#[cfg(not(target_os = "macos"))]
fn has_screen_capture_permission() -> bool {
    true
}

#[cfg(not(target_os = "macos"))]
fn request_screen_capture_permission() {}

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

/// Starts a capture in the background; ignored while another one is running.
pub fn start(app: AppHandle, action: CaptureAction) {
    tauri::async_runtime::spawn(run_capture(app, action));
}

/// The editor's capture buttons take a zone and open it unless they say otherwise.
#[tauri::command]
pub fn start_capture(app: AppHandle, action: Option<CaptureAction>) {
    start(app, action.unwrap_or(CaptureAction::Region));
}

async fn run_capture(app: AppHandle, action: CaptureAction) {
    let captures = app.state::<Captures>();
    if captures.in_progress.swap(true, Ordering::SeqCst) {
        return;
    }
    captures.copying.store(action.copies(), Ordering::SeqCst);
    let kind = action.kind();

    // Invariant: the screens are captured before anything activates Snapclone or moves the
    // focus (no `show`, `set_focus`, activation or overlay before the shot), so a menu or a
    // `<select>` open in another app stays open and shows in the capture. A shortcut press
    // does not activate the app; a hidden editor is left alone, and a visible one is only
    // ordered out, which does not activate the app either. Every capture kind keeps this:
    // the selector shoots the monitors before opening its overlays (which the scroll capture
    // also starts from), and the full-screen capture shows the editor only after the shot.
    // The user can keep Snapclone's windows in the shot ("Ocultar las ventanas al capturar").
    if app_settings::toggle(&app, Toggle::HideWindowsOnCapture) {
        let hidden = HiddenWindows {
            editor: editor_window::is_visible(&app),
            settings: app_settings::is_window_visible(&app),
        };
        *captures.hidden.lock().unwrap() = Some(hidden);
        // Ordered out even when it reports itself invisible: after ⌘H it still belongs on
        // screen, and unhiding the app below would bring it back into the shot.
        editor_window::hide(&app);
        app_settings::hide_window(&app);
        if hidden.editor || hidden.settings {
            tokio::time::sleep(HIDE_SETTLE).await;
        }
    }
    // Every kind opens its overlays (or shows the editor) in a hidden app otherwise.
    editor_window::unhide_without_activation(&app);

    // The own selector keeps `in_progress` set until the selection ends; see `selection_ended`.
    let own = tauri::async_runtime::spawn_blocking({
        let app = app.clone();
        move || match kind {
            CaptureKind::Screen => screen::capture().map(Some),
            _ => selector::open(&app, kind).map(|()| None),
        }
    })
    .await
    .unwrap_or_else(|error| Err(error.to_string()));
    match own {
        Ok(None) => return,
        Ok(Some((image, scale))) => {
            if let Err(error) = store_image(&app, image, scale).await {
                report_failure(&app, &error);
            }
            capture_ended(&app);
            return;
        }
        // No system tool can take a scroll capture, so its failure is shown as it is; only a
        // missing permission (checked, not assumed) gets its own explanation.
        Err(error) if kind == CaptureKind::Scroll => {
            let message = if has_screen_capture_permission() {
                format!("No se pudo empezar la captura con scroll: {error}")
            } else {
                "La captura con scroll necesita el permiso de Grabación de pantalla. Actívalo en Ajustes del Sistema › Privacidad y seguridad.".to_string()
            };
            report_failure(&app, &message);
            capture_ended(&app);
            return;
        }
        Err(error) => eprintln!("own capture unavailable, using screencapture: {error}"),
    }

    let result = match capture_dir(&app) {
        Ok(dir) => run_screencapture(&dir, kind.fallback_args()).await,
        Err(error) => Err(error),
    };
    match result {
        Ok(Some((id, path))) => {
            let scale = std::fs::read(&path).map_or(1.0, |bytes| png_scale(&bytes));
            app_settings::play_capture_sound(&app);
            deliver(&app, id, path, scale);
        }
        Ok(None) => {}
        Err(error) => report_failure(&app, &error),
    }
    capture_ended(&app);
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
    capture_ended(app);
}

/// Releases the capture and shows again the windows it hid, whether it was taken, failed or
/// was cancelled. A capture delivered to the editor already showed it; it is brought back
/// in front of the Preferences window. A copied capture never focuses the editor: it only
/// comes back, without the focus, if it was on screen before the shot.
fn capture_ended(app: &AppHandle) {
    let captures = app.state::<Captures>();
    let hidden = captures.hidden.lock().unwrap().take();
    let copying = captures.copying.swap(false, Ordering::SeqCst);
    captures.in_progress.store(false, Ordering::SeqCst);
    let Some(hidden) = hidden else { return };
    if hidden.settings {
        app_settings::show_window(app);
    }
    if copying {
        if hidden.editor {
            editor_window::show_without_focus(app);
        }
    } else if hidden.editor || editor_window::is_visible(app) {
        editor_window::show(app, false);
    }
}

/// Registers a finished capture, stores it in the history and hands it to the editor, which
/// opens it, or styles and copies it while staying hidden for "Capturar zona y copiar".
pub fn deliver(app: &AppHandle, id: String, path: PathBuf, scale: f64) {
    history::record(app, &id, &path, scale);
    // The history view only needs it later, so it never delays showing the capture.
    let source = path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if let Err(error) = images::write_thumbnail(&source) {
            eprintln!("thumbnail could not be written: {error}");
        }
    });
    let captures = app.state::<Captures>();
    captures.files.lock().unwrap().insert(id.clone(), path);
    let copy = captures.copying.load(Ordering::SeqCst);
    if copy {
        *captures.pending_copy.lock().unwrap() = Some(id.clone());
        watch_copy(app.clone(), id.clone());
    } else {
        editor_window::show(app, false);
    }
    if let Err(error) = app.emit_to(EDITOR_WINDOW, READY_EVENT, CaptureReady { id, scale, copy }) {
        eprintln!("capture-ready emit failed: {error}");
        if copy {
            finish_copy(app, &format!("No se pudo copiar la captura: {error}"));
        }
    }
}

/// A hidden editor that never answers (its page failed or was suspended) would leave the
/// capture neither copied nor shown; after `COPY_TIMEOUT` it is shown with the reason.
fn watch_copy(app: AppHandle, id: String) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(COPY_TIMEOUT).await;
        let captures = app.state::<Captures>();
        let expired = {
            let mut pending = captures.pending_copy.lock().unwrap();
            let expired = pending.as_deref() == Some(id.as_str());
            if expired {
                *pending = None;
            }
            expired
        };
        if expired {
            editor_window::notify(&app, "La captura tardaba demasiado en copiarse: revisa que esté abierta aquí y cópiala desde el editor", true);
        }
    });
}

fn finish_copy(app: &AppHandle, error: &str) {
    app.state::<Captures>().pending_copy.lock().unwrap().take();
    editor_window::notify(app, error, true);
}

/// The hidden editor styled and copied a capture (`error` is `None`), or could not. Success
/// shows the "copied" bezel without activating the app; a failure brings the editor forward
/// with the reason the page gives, so it is never lost behind a hidden window. A copy that
/// finishes after the time-out still says so.
#[tauri::command]
pub async fn copy_finished(app: AppHandle, id: String, error: Option<String>) {
    {
        let captures = app.state::<Captures>();
        let mut pending = captures.pending_copy.lock().unwrap();
        if pending.as_deref() == Some(id.as_str()) {
            *pending = None;
        }
    }
    match error {
        Some(error) => editor_window::notify(&app, &error, true),
        None => hud::show_copied(&app),
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

/// Encodes off the async runtime and hands the file to the normal capture flow.
pub async fn store_image(app: &AppHandle, image: RgbaImage, scale: f64) -> Result<(), String> {
    let dir = capture_dir(app)?;
    let id = uuid::Uuid::new_v4().to_string();
    let path = dir.join(format!("{id}.png"));
    let target = path.clone();
    tauri::async_runtime::spawn_blocking(move || images::write_png(&target, &image))
        .await
        .map_err(|error| error.to_string())??;
    app_settings::play_capture_sound(app);
    deliver(app, id, path, scale);
    Ok(())
}

/// Runs the system capture tool, the fallback when the own capture cannot read the screens.
/// Returns `None` when the user cancels with Esc.
async fn run_screencapture(dir: &Path, args: &[&str]) -> Result<Option<(String, PathBuf)>, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let path = dir.join(format!("{id}.png"));
    let status = tokio::process::Command::new("/usr/sbin/screencapture")
        .args(args)
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
pub fn png_scale(png: &[u8]) -> f64 {
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
/// Failures carry a plain-text reason the editor can show.
pub fn serve(app: &AppHandle, request: &http::Request<Vec<u8>>) -> http::Response<Vec<u8>> {
    let requested = request.uri().path().trim_start_matches('/');
    let served = match wallpaper::Variant::from_path(requested) {
        Some(variant) => wallpaper::serve(app, variant).map(|(bytes, kind, version)| (bytes, kind, Some(version))),
        None => {
            let path = match requested.strip_prefix(THUMBNAIL_PREFIX) {
                Some(id) => path_of(app, id).map(|path| thumbnail_or_capture(&path)),
                None => path_of(app, requested),
            };
            path.and_then(|path| std::fs::read(path).ok())
                .map(|bytes| (bytes, "image/png", None))
                .ok_or_else(|| "unknown capture".to_string())
        }
    };
    let response = http::Response::builder()
        .header("Access-Control-Allow-Origin", "*")
        .header("Access-Control-Expose-Headers", WALLPAPER_VERSION_HEADER);
    match served {
        Ok((bytes, kind, version)) => match version {
            Some(version) => response.header(WALLPAPER_VERSION_HEADER, version),
            None => response,
        }
        .header("Content-Type", kind)
        .body(bytes),
        Err(reason) => response.status(404).header("Content-Type", "text/plain; charset=utf-8").body(reason.into_bytes()),
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
