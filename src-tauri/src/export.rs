use std::{path::PathBuf, sync::Mutex};

use tauri::{
    State,
    ipc::{InvokeBody, Request},
};
use tauri_plugin_dialog::DialogExt;

/// The path the user picked in the save dialog. `save_image` only ever writes there,
/// so the webview cannot choose arbitrary destinations.
#[derive(Default)]
pub struct PendingSave(Mutex<Option<PathBuf>>);

/// Encoded image bytes arrive as a raw IPC body to avoid JSON-encoding megabytes.
fn raw_body<'a>(request: &'a Request<'_>) -> Result<&'a [u8], String> {
    match request.body() {
        InvokeBody::Raw(bytes) if !bytes.is_empty() => Ok(bytes),
        _ => Err("expected raw image bytes".into()),
    }
}

/// Puts the PNG on the general pasteboard as `public.png`, without re-encoding.
/// Kept synchronous so AppKit is called from the main thread.
#[tauri::command]
pub fn copy_png(request: Request<'_>) -> Result<(), String> {
    write_png_to_pasteboard(raw_body(&request)?)
}

/// Shows the native save dialog and remembers the choice. Returns the path so the
/// editor can pick PNG or JPEG from its extension, or `None` if the user cancelled.
#[tauri::command]
pub async fn choose_save_path(
    app: tauri::AppHandle,
    pending: State<'_, PendingSave>,
    default_name: String,
) -> Result<Option<String>, String> {
    let chosen = app
        .dialog()
        .file()
        .set_file_name(default_name)
        .add_filter("PNG", &["png"])
        .add_filter("JPEG", &["jpg", "jpeg"])
        .blocking_save_file()
        .map(|path| path.into_path().map_err(|error| error.to_string()))
        .transpose()?;
    let display = chosen.as_ref().map(|path| path.to_string_lossy().into_owned());
    *pending.0.lock().unwrap() = chosen;
    Ok(display)
}

/// Writes the encoded image to the path from the last `choose_save_path`, off the main thread.
#[tauri::command]
pub async fn save_image(pending: State<'_, PendingSave>, request: Request<'_>) -> Result<(), String> {
    let path = pending.0.lock().unwrap().take().ok_or("no save destination was chosen")?;
    let bytes = raw_body(&request)?.to_vec();
    tauri::async_runtime::spawn_blocking(move || std::fs::write(path, bytes))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}

#[cfg(target_os = "macos")]
fn write_png_to_pasteboard(bytes: &[u8]) -> Result<(), String> {
    use objc2_app_kit::{NSPasteboard, NSPasteboardTypePNG};
    use objc2_foundation::NSData;

    let data = NSData::with_bytes(bytes);
    let pasteboard = NSPasteboard::generalPasteboard();
    pasteboard.clearContents();
    // SAFETY: AppKit defines this constant for the lifetime of the process.
    let png_type = unsafe { NSPasteboardTypePNG };
    if pasteboard.setData_forType(Some(&data), png_type) {
        Ok(())
    } else {
        Err("the pasteboard rejected the image".into())
    }
}

/// Replaces the pasteboard contents with plain text. Call it on the main thread.
#[cfg(target_os = "macos")]
pub fn write_text_to_pasteboard(text: &str) -> Result<(), String> {
    use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
    use objc2_foundation::NSString;

    let pasteboard = NSPasteboard::generalPasteboard();
    pasteboard.clearContents();
    // SAFETY: AppKit defines this constant for the lifetime of the process.
    let string_type = unsafe { NSPasteboardTypeString };
    if pasteboard.setString_forType(&NSString::from_str(text), string_type) {
        Ok(())
    } else {
        Err("the pasteboard rejected the text".into())
    }
}

#[cfg(not(target_os = "macos"))]
pub fn write_text_to_pasteboard(_text: &str) -> Result<(), String> {
    Err("copying text is only implemented on macOS".into())
}

#[cfg(not(target_os = "macos"))]
fn write_png_to_pasteboard(_bytes: &[u8]) -> Result<(), String> {
    Err("copying images is only implemented on macOS".into())
}
