use percent_encoding::percent_decode_str;
use tauri::ipc::{InvokeBody, Request};

/// Encoded image bytes arrive as a raw IPC body to avoid JSON-encoding megabytes.
fn raw_body<'a>(request: &'a Request<'_>) -> Result<&'a [u8], String> {
    match request.body() {
        InvokeBody::Raw(bytes) if !bytes.is_empty() => Ok(bytes),
        _ => Err("expected raw image bytes".into()),
    }
}

/// Puts the PNG on the general pasteboard as `public.png`, without re-encoding.
#[tauri::command]
pub fn copy_png(request: Request<'_>) -> Result<(), String> {
    write_png_to_pasteboard(raw_body(&request)?)
}

/// Writes already-encoded PNG/JPEG bytes to the path chosen in the save dialog.
/// The path travels percent-encoded in the `path` header because headers must be ASCII.
#[tauri::command]
pub fn save_image(request: Request<'_>) -> Result<(), String> {
    let bytes = raw_body(&request)?;
    let encoded = request
        .headers()
        .get("path")
        .and_then(|value| value.to_str().ok())
        .ok_or("missing path header")?;
    let path = percent_decode_str(encoded)
        .decode_utf8()
        .map_err(|error| error.to_string())?;
    std::fs::write(path.as_ref(), bytes).map_err(|error| error.to_string())
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

#[cfg(not(target_os = "macos"))]
fn write_png_to_pasteboard(_bytes: &[u8]) -> Result<(), String> {
    Err("copying images is only implemented on macOS".into())
}
