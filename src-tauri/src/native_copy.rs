//! "Capturar zona y copiar" without the webview: the zone is styled in Rust, put on the
//! clipboard and confirmed before it is even written to disk. The hidden editor's copy is
//! the fallback whenever the native route does not apply or fails, so no capture is lost.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use serde_json::Value;
use tauri::AppHandle;
use xcap::image::RgbaImage;

use crate::{app_settings, capture, compose, editor_window, export, hud, images, prefs};

const AUTO_REDACT_KEY: &str = "autoRedact";
const REDACT_KEY: &str = "redact";
/// The editor's automatic redaction categories; each one is on unless stored as `false`.
const REDACT_CATEGORIES: [&str; 4] = ["email", "credit-card", "ip-address", "api-key"];
/// AppKit is reached on the main thread; this bounds the wait if that thread is busy.
const MAIN_THREAD_TIMEOUT: Duration = Duration::from_secs(2);
const STORE_FAILED: &str = "Copiada, pero no se pudo guardar en el historial";

/// Whether the editor would redact a new capture, with its rules and defaults: the main
/// switch is on unless stored as `false`, and so is every category.
pub fn redaction_enabled(auto_redact: Option<&Value>, redact: Option<&Value>) -> bool {
    let enabled = auto_redact.and_then(Value::as_bool).unwrap_or(true);
    let category_on = |id: &str| redact.and_then(Value::as_object).and_then(|categories| categories.get(id)).and_then(Value::as_bool).unwrap_or(true);
    enabled && REDACT_CATEGORIES.iter().any(|id| category_on(id))
}

/// The native route needs the copy action, macOS, and no automatic redaction, which only
/// the editor can apply (OCR and pixelation).
pub fn native_route(copying: bool, macos: bool, redaction: bool) -> bool {
    copying && macos && !redaction
}

/// Stores a zone or window capture; when it is being copied and the native route applies,
/// it is styled and copied first, falling back to the editor's copy on any failure.
pub async fn copy_or_store(app: &AppHandle, image: RgbaImage, scale: f64) -> Result<(), String> {
    let redaction = redaction_enabled(prefs::read::<Value>(app, AUTO_REDACT_KEY).as_ref(), prefs::read::<Value>(app, REDACT_KEY).as_ref());
    if !native_route(capture::is_copying(app), cfg!(target_os = "macos"), redaction) {
        return capture::store_image(app, image, scale).await;
    }
    let (image, copied) = copy(app, image, scale).await?;
    if let Err(error) = copied {
        eprintln!("native copy failed, the editor copies instead: {error}");
        return capture::store_image(app, image, scale).await;
    }
    // The image is on the clipboard and the user was told so; a disk failure from here on
    // only costs the history entry and must not read as a failed capture.
    if let Err(error) = capture::store_copied(app, image, scale).await {
        editor_window::notify(app, &format!("{STORE_FAILED}: {error}"), true);
    }
    Ok(())
}

/// Composes and encodes off the async runtime, writes the clipboard on the main thread,
/// then applies "Cerrar la captura después de copiarla", plays the capture sound and shows
/// the "copied" bezel. The image comes back for storing whatever happened; the outer error
/// only means the worker itself was lost.
async fn copy(app: &AppHandle, image: RgbaImage, scale: f64) -> Result<(RgbaImage, Result<(), String>), String> {
    let worker = app.clone();
    let (image, png) = tauri::async_runtime::spawn_blocking(move || {
        let png = styled_png(&worker, &image, scale);
        (image, png)
    })
    .await
    .map_err(|error| format!("the native copy worker failed: {error}"))?;
    let copied = match png {
        Ok(png) => write_clipboard(app, png).await,
        Err(error) => Err(error),
    };
    if copied.is_ok() {
        capture::copied_natively(app);
        app_settings::play_capture_sound(app);
        hud::show_copied(app);
    }
    Ok((image, copied))
}

/// Writes the pasteboard on the main thread and waits a bounded time. A write still queued
/// when the wait gives up is cancelled, so it never lands after the editor's own copy.
async fn write_clipboard(app: &AppHandle, png: Vec<u8>) -> Result<(), String> {
    let cancelled = Arc::new(AtomicBool::new(false));
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let queued = Arc::clone(&cancelled);
    app.run_on_main_thread(move || {
        if !queued.load(Ordering::SeqCst) {
            let _ = sender.send(export::write_png_to_pasteboard(&png));
        }
    })
    .map_err(|error| error.to_string())?;
    match tokio::time::timeout(MAIN_THREAD_TIMEOUT, receiver).await {
        Ok(written) => written.map_err(|_| "the clipboard write was dropped".to_string())?,
        Err(_) => {
            cancelled.store(true, Ordering::SeqCst);
            Err("the main thread did not write the clipboard in time".into())
        }
    }
}

/// The capture styled with the editor's stored settings, as PNG. A wallpaper background
/// that cannot be read fails, so the editor's copy reports why instead of using a stand-in.
fn styled_png(app: &AppHandle, image: &RgbaImage, scale: f64) -> Result<Vec<u8>, String> {
    let (style, balance) = compose::stored_settings(app);
    #[cfg(target_os = "macos")]
    let wallpaper = match style.background {
        compose::Background::Wallpaper => {
            Some(compose::current_wallpaper(app).map_err(|error| format!("desktop picture unavailable: {error}"))?)
        }
        _ => None,
    };
    #[cfg(not(target_os = "macos"))]
    let wallpaper: Option<Arc<compose::BlurredWallpaper>> = None;
    let composed = compose::compose(image, scale, &style, balance, wallpaper.as_deref()).map_err(|error| error.to_string())?;
    images::encode_png(&composed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn redaction_follows_the_editor_defaults() {
        assert!(redaction_enabled(None, None), "a new install redacts every category");
        assert!(!redaction_enabled(Some(&json!(false)), None));
        assert!(redaction_enabled(Some(&json!("off")), None), "an invalid switch keeps its default");
        let none = json!({"email": false, "credit-card": false, "ip-address": false, "api-key": false});
        assert!(!redaction_enabled(Some(&json!(true)), Some(&none)), "every category off");
        let one = json!({"email": false, "credit-card": false, "ip-address": false});
        assert!(redaction_enabled(Some(&json!(true)), Some(&one)), "a missing category is on");
        let invalid = json!({"email": false, "credit-card": false, "ip-address": false, "api-key": "no"});
        assert!(redaction_enabled(None, Some(&invalid)), "a non-boolean category is on");
        assert!(redaction_enabled(None, Some(&json!([]))), "an invalid object means the defaults");
    }

    #[test]
    fn native_route_only_for_unredacted_copies_on_macos() {
        assert!(native_route(true, true, false));
        assert!(!native_route(false, true, false), "opening in the editor");
        assert!(!native_route(true, true, true), "automatic redaction needs the editor");
        assert!(!native_route(true, false, false), "other platforms");
    }
}
