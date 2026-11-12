//! Native compositor: turns a capture and the stored style into the final image the editor
//! would export (no annotations, no selection), without going through the webview. It
//! follows the editor's renderer rule by rule; the geometry, the style parsing and the
//! balance analysis are pure and tested, the drawing goes through CoreGraphics.

mod balance;
#[cfg(target_os = "macos")]
mod canvas;
#[cfg(target_os = "macos")]
mod draw;
mod layout;
mod style;
#[cfg(all(test, target_os = "macos"))]
mod tests;
mod wallpaper;

use std::fmt;

use serde_json::Value;
use tauri::AppHandle;
use xcap::image::RgbaImage;

pub use style::{Background, StoredStyle};
#[cfg(target_os = "macos")]
pub use wallpaper::current as current_wallpaper;
pub use wallpaper::{BlurredWallpaper, BlurredWallpaperCache};

const STYLE_KEY: &str = "style";
const BALANCE_KEY: &str = "balance";

#[derive(Debug, PartialEq, Eq)]
pub enum ComposeError {
    /// This platform or input cannot be composed natively; the caller uses the editor.
    #[cfg_attr(target_os = "macos", allow(dead_code, reason = "only other platforms construct it so far"))]
    Unsupported(String),
    /// Composing was attempted and failed.
    Failed(String),
}

impl fmt::Display for ComposeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported(reason) => write!(formatter, "native composition unsupported: {reason}"),
            Self::Failed(reason) => write!(formatter, "native composition failed: {reason}"),
        }
    }
}

impl std::error::Error for ComposeError {}

/// The composed image for `capture` taken at display `scale`. With `balance`, a capture on a
/// flat backdrop is re-centred first, as the editor does when its toggle is on; one that
/// cannot be balanced is composed as it is. A wallpaper background without a `wallpaper`
/// shows the first gradient, as the editor does until macOS hands over the picture.
pub fn compose(
    capture: &RgbaImage,
    scale: f64,
    style: &StoredStyle,
    balance: bool,
    wallpaper: Option<&BlurredWallpaper>,
) -> Result<RgbaImage, ComposeError> {
    if capture.width() == 0 || capture.height() == 0 {
        return Err(ComposeError::Failed("the capture is empty".into()));
    }
    if !scale.is_finite() || scale <= 0.0 {
        return Err(ComposeError::Failed(format!("invalid display scale {scale}")));
    }
    #[cfg(target_os = "macos")]
    {
        let balanced = balance.then(|| balanced(capture)).flatten();
        draw::compose(balanced.as_ref().unwrap_or(capture), scale, style, wallpaper).map_err(ComposeError::Failed)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (style, balance, wallpaper);
        Err(ComposeError::Unsupported("only macOS composes natively".into()))
    }
}

/// The balanced capture, or `None` when its border is not a flat colour or it has no content.
#[cfg(target_os = "macos")]
fn balanced(capture: &RgbaImage) -> Option<RgbaImage> {
    match balance::analyze_balance(&balance::canvas_read_back(capture)) {
        balance::BalanceResult::Balanced(plan) => Some(balance::balanced_image(capture, &plan)),
        balance::BalanceResult::NotUniform | balance::BalanceResult::NoContent => None,
    }
}

/// The editor's saved style and balance toggle, with the editor's defaults for anything
/// missing or invalid.
pub fn stored_settings(app: &AppHandle) -> (StoredStyle, bool) {
    let style = crate::prefs::read::<Value>(app, STYLE_KEY).as_ref().and_then(StoredStyle::parse).unwrap_or_default();
    let balance = crate::prefs::read::<Value>(app, BALANCE_KEY).and_then(|value| value.as_bool()).unwrap_or(false);
    (style, balance)
}

/// Composes a capture from the history with the current settings and writes it to
/// `<app data>/compose-debug/<id>.png`, to compare with the editor's export. Debug builds only.
#[cfg(debug_assertions)]
#[tauri::command]
pub async fn compose_debug(app: AppHandle, id: String) -> Result<String, String> {
    // The wallpaper is read through the main thread, so this must run off it.
    tauri::async_runtime::spawn_blocking(move || compose_debug_blocking(&app, &id)).await.map_err(|error| error.to_string())?
}

#[cfg(debug_assertions)]
fn compose_debug_blocking(app: &AppHandle, id: &str) -> Result<String, String> {
    use tauri::Manager;

    // The id names the output file, so it must not carry a path.
    if id.is_empty() || !id.chars().all(|character| character.is_ascii_alphanumeric() || character == '-') {
        return Err("invalid capture id".into());
    }
    let path = crate::capture::path_of(app, id).ok_or("unknown capture")?;
    let bytes = std::fs::read(&path).map_err(|error| format!("the capture could not be read: {error}"))?;
    let scale = crate::history::scale_of(app, id).unwrap_or_else(|| crate::capture::png_scale(&bytes));
    let capture = xcap::image::load_from_memory(&bytes).map_err(|error| error.to_string())?.to_rgba8();
    let (style, balance) = stored_settings(app);
    #[cfg(target_os = "macos")]
    let wallpaper = match style.background {
        Background::Wallpaper => Some(current_wallpaper(app)?),
        _ => None,
    };
    #[cfg(not(target_os = "macos"))]
    let wallpaper: Option<std::sync::Arc<BlurredWallpaper>> = None;
    let composed = compose(&capture, scale, &style, balance, wallpaper.as_deref()).map_err(|error| error.to_string())?;
    let dir = app.path().app_data_dir().map_err(|error| error.to_string())?.join("compose-debug");
    std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    let output = dir.join(format!("{id}.png"));
    crate::images::write_png(&output, &composed)?;
    Ok(output.to_string_lossy().into_owned())
}
