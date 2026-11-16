use std::{path::PathBuf, sync::Mutex};

#[cfg(target_os = "windows")]
use tauri::Manager;
use tauri::{State, ipc::Request};
use tauri_plugin_dialog::DialogExt;

use crate::{
    app_settings::{self, Toggle},
    editor_window,
    output::{self, raw_body},
};

/// The path the user picked in the save dialog. `save_image` only ever writes there,
/// so the webview cannot choose arbitrary destinations.
#[derive(Default)]
pub struct PendingSave(Mutex<Option<PathBuf>>);

/// Puts the PNG on the general pasteboard as `public.png`, without re-encoding, then closes
/// the editor when "Cerrar la captura después de copiarla" is on. The setting is read from
/// the settings store in memory. Kept synchronous so AppKit is called from the main thread.
#[cfg(not(target_os = "windows"))]
#[tauri::command]
pub fn copy_png(app: tauri::AppHandle, request: Request<'_>) -> Result<(), String> {
    write_png_to_pasteboard(raw_body(&request)?)?;
    close_after_copy(&app);
    Ok(())
}

/// Hides the editor when "Cerrar la captura después de copiarla" is on; returns whether it
/// did. Shared by the editor's copy and the one Rust makes for "Capturar zona y copiar".
pub fn close_after_copy(app: &tauri::AppHandle) -> bool {
    let close = app_settings::toggle(app, Toggle::CloseAfterCopy);
    if close {
        editor_window::hide(app);
    }
    close
}

/// Windows needs its own PNG → RGBA decode before the pasteboard accepts it (`arboard` takes
/// raw pixels, not PNG bytes), which costs real CPU/memory on a scroll-capture-sized image —
/// unlike AppKit, `arboard` has no main-thread requirement, so that work runs off it here.
#[cfg(target_os = "windows")]
#[tauri::command]
pub async fn copy_png(app: tauri::AppHandle, request: Request<'_>) -> Result<(), String> {
    use std::sync::atomic::Ordering;
    let bytes = raw_body(&request)?.to_vec();
    // Generation is taken at the START: any copy that starts after this one invalidates it.
    let generation = app.state::<CopyGeneration>().next.fetch_add(1, Ordering::SeqCst);
    let worker_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        write_png_in_order(&worker_app.state::<CopyGeneration>(), generation, &bytes)
    })
    .await
    .map_err(|error| error.to_string())??;
    close_after_copy(&app);
    Ok(())
}

/// Shows the native save dialog, starting in the save folder with the default name, and
/// remembers the choice. Returns the path so the editor can pick PNG or JPEG from its
/// extension, or `None` if the user cancelled. `datetime` is the editor's local export time.
#[tauri::command]
pub async fn choose_save_path(
    app: tauri::AppHandle,
    pending: State<'_, PendingSave>,
    datetime: String,
) -> Result<Option<String>, String> {
    let (name, folder) = output::save_dialog_defaults(&app, &datetime)?;
    let mut dialog = app.dialog().file().set_file_name(name);
    if let Some(folder) = folder {
        dialog = dialog.set_directory(folder);
    }
    let chosen = dialog
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
pub async fn save_image(app: tauri::AppHandle, pending: State<'_, PendingSave>, request: Request<'_>) -> Result<(), String> {
    let path = pending.0.lock().unwrap().take().ok_or("no save destination was chosen")?;
    output::write_file(path.clone(), raw_body(&request)?.to_vec()).await?;
    output::remember_saved(&app, path);
    Ok(())
}

/// Replaces the pasteboard contents with a PNG. Call it on the main thread.
#[cfg(target_os = "macos")]
pub fn write_png_to_pasteboard(bytes: &[u8]) -> Result<(), String> {
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

/// Tracks which copy is the most recent one still writing. Without this, a slow copy of a huge
/// scroll capture could finish after a later, smaller copy and overwrite it on the pasteboard.
/// Registered once with `.manage(CopyGeneration::default())` in `lib.rs`, Windows only.
#[cfg(target_os = "windows")]
#[derive(Default)]
pub struct CopyGeneration {
    next: std::sync::atomic::AtomicU64,
    /// Serialises the "am I still the latest?" check with the pasteboard write itself, so no
    /// other worker's check can land in between them.
    write: Mutex<()>,
}

#[cfg(target_os = "windows")]
fn decode_rgba(bytes: &[u8]) -> Result<xcap::image::RgbaImage, String> {
    // The decoder's default `max_alloc` (512MiB) is lower than what this app otherwise allows.
    let mut reader = xcap::image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| error.to_string())?;
    reader.limits(crate::import::decode_limits());
    // `into_rgba8` reuses the decoded buffer in place when the source is already RGBA8 (the
    // common case here, since captures are encoded as RGBA PNGs); `to_rgba8` would always
    // allocate and copy a second buffer on top of the first.
    Ok(reader.decode().map_err(|error| error.to_string())?.into_rgba8())
}

/// Runs inside `spawn_blocking`. `generation` was taken when this copy started. `png` is used
/// as-is for the `"PNG"` clipboard format (no re-encode); it is also decoded once for the
/// `CF_DIBV5` fallback that apps without PNG support (Word, Paint) read instead.
#[cfg(target_os = "windows")]
fn write_png_in_order(state: &CopyGeneration, generation: u64, png: &[u8]) -> Result<(), String> {
    use std::sync::atomic::Ordering;
    // Cheap check before paying for decode/DIB conversion: if a newer copy already started
    // (common when the user copies several captures in quick succession), this one is already
    // obsolete and there is no point decoding a multi-megabyte image just to discard it.
    if state.next.load(Ordering::SeqCst) != generation + 1 {
        return Ok(());
    }
    let rgba = decode_rgba(png)?;
    let dib = windows_clipboard::rgba_to_dibv5(&rgba);
    let _write = state.write.lock().unwrap();
    if state.next.load(Ordering::SeqCst) != generation + 1 {
        // A later copy started while this one was decoding: still obsolete, dropped silently.
        return Ok(());
    }
    windows_clipboard::write_image(png, &dib)
}

/// Replaces the pasteboard contents with plain text, excluded from Clipboard History and Cloud
/// Clipboard like every other copy Snapclone makes on Windows (see Security Considerations in
/// the Windows port plan: a screenshot can carry sensitive data even after auto-redaction).
/// `arboard`'s `text()` sets these exclusions correctly (unlike `image()` — see
/// `windows_clipboard`), so it is kept for text.
#[cfg(target_os = "windows")]
pub fn write_text_to_pasteboard(text: &str) -> Result<(), String> {
    use arboard::SetExtWindows;
    arboard::Clipboard::new()
        .map_err(|error| error.to_string())?
        .set()
        .exclude_from_history()
        .exclude_from_cloud()
        .text(text)
        .map_err(|error| error.to_string())
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn write_text_to_pasteboard(_text: &str) -> Result<(), String> {
    Err("copying text is only implemented on macOS and Windows".into())
}

/// Only macOS calls this (`native_copy`'s native route is macOS-only); Windows copies through
/// `copy_png`, which orders concurrent writes, so there is nothing to do here elsewhere.
#[cfg(not(target_os = "macos"))]
pub fn write_png_to_pasteboard(_bytes: &[u8]) -> Result<(), String> {
    Err("copying images is only implemented on macOS".into())
}

/// Writes an image to the Windows clipboard directly, bypassing `arboard` for this one format.
/// `arboard` 3.7.0's `Set::image()` on Windows never calls its own `add_clipboard_exclusions` —
/// only `text()`/`html()`/`file_list()` do (confirmed in its source) — so `.exclude_from_history()`/
/// `.exclude_from_cloud()` are silently ignored for images: every screenshot copy would persist in
/// Clipboard History (Win+V) and sync via Cloud Clipboard, defeating this phase's whole privacy
/// requirement. Writing the formats ourselves, in one clipboard session that also sets the two
/// exclusion formats before closing, fixes that — and as a side effect avoids re-encoding a PNG
/// `arboard` would have decoded again internally: the original bytes are used for `"PNG"` as-is.
#[cfg(target_os = "windows")]
mod windows_clipboard {
    use windows::{
        Win32::{
            Foundation::{GlobalFree, HANDLE},
            Graphics::Gdi::{BI_BITFIELDS, BITMAPV5HEADER},
            System::{
                DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW, SetClipboardData},
                Memory::{GHND, GlobalAlloc, GlobalLock, GlobalUnlock},
                Ole::CF_DIBV5,
            },
        },
        core::HSTRING,
    };

    struct ClipboardGuard;

    impl ClipboardGuard {
        /// A few short retries: another app can briefly be holding the clipboard too.
        fn open() -> Result<Self, String> {
            for attempt in 0..5 {
                // SAFETY: no window needs to own the clipboard just to write to it.
                if unsafe { OpenClipboard(None) }.is_ok() {
                    return Ok(Self);
                }
                if attempt < 4 {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
            }
            Err("No se pudo acceder al portapapeles".into())
        }
    }

    impl Drop for ClipboardGuard {
        fn drop(&mut self) {
            // SAFETY: only reached after a successful `OpenClipboard` on this same thread.
            let _ = unsafe { CloseClipboard() };
        }
    }

    /// Copies `bytes` into a fresh global block and hands it to the clipboard under `format`.
    /// Ownership of the block passes to the system on success, as `SetClipboardData` requires;
    /// it is freed here on every failure path instead.
    fn set_global(format: u32, bytes: &[u8]) -> Result<(), String> {
        // SAFETY: `GHND` gives a moveable, zero-initialized block; the result is checked below.
        let handle = unsafe { GlobalAlloc(GHND, bytes.len()) }.map_err(|error| error.to_string())?;
        // SAFETY: `handle` was just allocated above and is not yet shared with anything else.
        let ptr = unsafe { GlobalLock(handle) };
        if ptr.is_null() {
            // SAFETY: freeing a block that was never locked or handed to the clipboard.
            unsafe { let _ = GlobalFree(Some(handle)); }
            return Err("No se pudo preparar el portapapeles".into());
        }
        // SAFETY: `ptr` is valid for `bytes.len()` bytes: it was just allocated with exactly
        // that size and locked immediately above.
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr.cast(), bytes.len()) };
        // SAFETY: matches the successful lock above.
        let _ = unsafe { GlobalUnlock(handle) };
        // SAFETY: `handle` is a valid, unlocked global block; the clipboard takes ownership of
        // it on success, matching the API contract `SetClipboardData` documents.
        if unsafe { SetClipboardData(format, Some(HANDLE(handle.0))) }.is_err() {
            // SAFETY: the clipboard did not take ownership (the call above failed).
            unsafe { let _ = GlobalFree(Some(handle)); }
            return Err("El portapapeles rechazó la imagen".into());
        }
        Ok(())
    }

    /// Marks whatever is currently on the clipboard as excluded from Clipboard History and
    /// Cloud Clipboard: the two system-defined formats Windows itself checks for, each present
    /// (with any content, including empty) meaning "exclude this".
    fn exclude_from_history_and_cloud() {
        for name in ["CanIncludeInClipboardHistory", "CanUploadToCloudClipboard"] {
            // SAFETY: a plain format-name registration, valid regardless of clipboard state.
            let format = unsafe { RegisterClipboardFormatW(&HSTRING::from(name)) };
            let _ = set_global(format, &0u32.to_le_bytes());
        }
    }

    /// `rgba`'s pixels, as the bottom-up BGRA `BITMAPV5HEADER` body Word/Paint/etc. expect from
    /// `CF_DIBV5` (a negative height, which would avoid the flip, is rejected by Word/WordPad).
    pub fn rgba_to_dibv5(rgba: &xcap::image::RgbaImage) -> Vec<u8> {
        const LCS_SRGB: u32 = 0x7352_4742; // missing from this crate's Gdi bindings
        let (width, height) = (rgba.width(), rgba.height());
        let header = BITMAPV5HEADER {
            bV5Size: size_of::<BITMAPV5HEADER>() as u32,
            bV5Width: width as i32,
            bV5Height: height as i32,
            bV5Planes: 1,
            bV5BitCount: 32,
            bV5Compression: BI_BITFIELDS,
            bV5SizeImage: width * height * 4,
            bV5RedMask: 0x00FF_0000,
            bV5GreenMask: 0x0000_FF00,
            bV5BlueMask: 0x0000_00FF,
            bV5AlphaMask: 0xFF00_0000,
            bV5CSType: LCS_SRGB,
            bV5Intent: 4, // LCS_GM_IMAGES
            ..Default::default()
        };
        // SAFETY: `BITMAPV5HEADER` is a plain `#[repr(C)]` struct of fixed-size integer fields.
        let header_bytes = unsafe { std::slice::from_raw_parts((&raw const header).cast::<u8>(), size_of::<BITMAPV5HEADER>()) };
        let mut dib = Vec::with_capacity(header_bytes.len() + rgba.len());
        dib.extend_from_slice(header_bytes);
        for y in (0..height).rev() {
            // Bottom-up: the last image row is written first.
            for x in 0..width {
                let [r, g, b, a] = rgba.get_pixel(x, y).0;
                dib.extend_from_slice(&[b, g, r, a]);
            }
        }
        dib
    }

    /// `png` is written as-is for the `"PNG"` format (first, so apps offered both prefer it);
    /// `dibv5` is the `CF_DIBV5` fallback from [`rgba_to_dibv5`]. Both formats, and the two
    /// exclusion formats, are set in one `OpenClipboard` session — the exclusions must be
    /// present before `CloseClipboard` for the shell to honour them.
    pub fn write_image(png: &[u8], dibv5: &[u8]) -> Result<(), String> {
        let _clipboard = ClipboardGuard::open()?;
        // SAFETY: the clipboard is open, owned by this thread.
        unsafe { EmptyClipboard() }.map_err(|error| error.to_string())?;
        // SAFETY: a plain format-name registration.
        let png_format = unsafe { RegisterClipboardFormatW(&HSTRING::from("PNG")) };
        set_global(png_format, png)?;
        set_global(CF_DIBV5.0 as u32, dibv5)?;
        exclude_from_history_and_cloud();
        Ok(())
    }
}
