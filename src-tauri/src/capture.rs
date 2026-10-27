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

pub const EDITOR_WINDOW: &str = "main";
const READY_EVENT: &str = "capture-ready";

/// Captured files by id, served to the editor through the `capture://` scheme.
#[derive(Default)]
pub struct Captures {
    files: Mutex<HashMap<String, PathBuf>>,
    in_progress: AtomicBool,
}

#[derive(Clone, Serialize)]
struct CaptureReady {
    id: String,
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

    let editor = app.get_webview_window(EDITOR_WINDOW);
    if let Some(window) = &editor {
        let _ = window.hide();
    }

    let result = match capture_dir(&app) {
        Ok(dir) => run_screencapture(&dir).await,
        Err(error) => Err(error),
    };
    captures.in_progress.store(false, Ordering::SeqCst);

    match result {
        Ok(Some((id, path))) => {
            captures.files.lock().unwrap().insert(id.clone(), path);
            if let Some(window) = &editor {
                let _ = window.show();
                let _ = window.set_focus();
            }
            if let Err(error) = app.emit_to(EDITOR_WINDOW, READY_EVENT, CaptureReady { id }) {
                eprintln!("capture-ready emit failed: {error}");
            }
        }
        Ok(None) => {}
        Err(error) => eprintln!("capture failed: {error}"),
    }
}

fn capture_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_cache_dir()
        .map_err(|error| error.to_string())?
        .join("captures");
    std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    Ok(dir)
}

/// Runs the native interactive selector. Returns `None` when the user cancels with Esc.
async fn run_screencapture(dir: &Path) -> Result<Option<(String, PathBuf)>, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let path = dir.join(format!("{id}.png"));
    let status = tokio::process::Command::new("/usr/sbin/screencapture")
        .args(["-i", "-x"])
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

/// Serves `capture://localhost/<id>` with CORS so the editor canvas stays exportable.
pub fn serve(app: &AppHandle, request: &http::Request<Vec<u8>>) -> http::Response<Vec<u8>> {
    let id = request.uri().path().trim_start_matches('/');
    let path = app.state::<Captures>().files.lock().unwrap().get(id).cloned();
    let response = http::Response::builder().header("Access-Control-Allow-Origin", "*");
    match path.and_then(|path| std::fs::read(path).ok()) {
        Some(bytes) => response.header("Content-Type", "image/png").body(bytes),
        None => response.status(404).body(Vec::new()),
    }
    .expect("static response parts are valid")
}
