//! Own region selector: every monitor is captured with `xcap` when the shortcut fires, and
//! each one gets a borderless overlay window showing that frozen image dimmed. Dragging
//! picks a rectangle, which is cropped from the monitor image here in Rust; with Space the
//! window under the cursor can be captured instead.

use std::{path::Path, sync::Mutex};

use serde::{Deserialize, Serialize};
use tauri::{
    AppHandle, Manager, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
    ipc::Response,
};
use xcap::image::{ImageEncoder, RgbaImage, codecs::png};

use crate::capture;

const WINDOW_PREFIX: &str = "selector-";
const PAGE: &str = "selector.html";
/// Menu bar items, the Dock and similar chrome are not windows anyone means to capture.
const IGNORED_OWNERS: &[&str] = &["Window Server", "Dock"];
const MIN_WINDOW_SIZE: u32 = 40;

/// The frozen screen state of the selection in progress, if any.
#[derive(Default)]
pub struct Selection(Mutex<Option<Session>>);

struct Session {
    monitors: Vec<MonitorShot>,
    windows: Vec<WindowInfo>,
}

struct MonitorShot {
    /// Global position and size in points, as the overlay window is laid out.
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    image: RgbaImage,
}

impl MonitorShot {
    /// Pixels per point, from the actual image rather than the reported display mode.
    fn scale(&self) -> f64 {
        f64::from(self.image.width()) / f64::from(self.width.max(1))
    }
}

/// A window that can be picked in window mode, in global points. Front-most first.
#[derive(Clone, Serialize)]
struct WindowInfo {
    id: u32,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

/// A rectangle in points; negative sizes (a drag up or left) are accepted.
#[derive(Clone, Copy, Debug, Deserialize)]
pub struct Rect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

/// Pixel rectangle inside an image: x, y, width, height.
type PixelRect = (u32, u32, u32, u32);

/// Converts a selection in points to the pixels of an image at `scale` pixels per point,
/// rounding outwards so nothing selected is lost and clamping to the image. `None` when
/// the result would be empty or the input is not a real rectangle.
pub fn crop_rect(rect: Rect, scale: f64, image_width: u32, image_height: u32) -> Option<PixelRect> {
    let values = [rect.x, rect.y, rect.width, rect.height, scale];
    if values.iter().any(|value| !value.is_finite()) || scale <= 0.0 {
        return None;
    }
    let clamp = |value: f64, max: u32| value.clamp(0.0, f64::from(max)) as u32;
    let (x0, x1) = (rect.x.min(rect.x + rect.width), rect.x.max(rect.x + rect.width));
    let (y0, y1) = (rect.y.min(rect.y + rect.height), rect.y.max(rect.y + rect.height));
    let left = clamp((x0 * scale).floor(), image_width);
    let top = clamp((y0 * scale).floor(), image_height);
    let right = clamp((x1 * scale).ceil(), image_width);
    let bottom = clamp((y1 * scale).ceil(), image_height);
    (right > left && bottom > top).then_some((left, top, right - left, bottom - top))
}

/// Captures every monitor and opens the overlays. On error nothing is left open, so the
/// caller can fall back to the system selector.
pub fn open(app: &AppHandle) -> Result<(), String> {
    let monitors = xcap::Monitor::all().map_err(|error| error.to_string())?;
    if monitors.is_empty() {
        return Err("no monitors found".into());
    }
    let shots = monitors
        .iter()
        .map(|monitor| {
            Ok(MonitorShot {
                x: monitor.x()?,
                y: monitor.y()?,
                width: monitor.width()?,
                height: monitor.height()?,
                image: monitor.capture_image()?,
            })
        })
        .collect::<xcap::XCapResult<Vec<_>>>()
        .map_err(|error| error.to_string())?;
    // Window mode is optional: without the list, region selection still works.
    let windows = list_windows().unwrap_or_else(|error| {
        eprintln!("window list unavailable: {error}");
        Vec::new()
    });

    let layout: Vec<(i32, i32, u32, u32)> = shots.iter().map(|shot| (shot.x, shot.y, shot.width, shot.height)).collect();
    *app.state::<Selection>().0.lock().unwrap() = Some(Session { monitors: shots, windows });

    for (index, (x, y, width, height)) in layout.into_iter().enumerate() {
        if let Err(error) = open_overlay(app, index, x, y, width, height) {
            close_overlays(app);
            app.state::<Selection>().0.lock().unwrap().take();
            return Err(error.to_string());
        }
    }
    Ok(())
}

fn list_windows() -> xcap::XCapResult<Vec<WindowInfo>> {
    let own_pid = std::process::id();
    let mut windows = Vec::new();
    for window in xcap::Window::all()? {
        let Ok(owner) = window.app_name() else { continue };
        if IGNORED_OWNERS.contains(&owner.as_str()) || window.pid()? == own_pid || window.is_minimized()? {
            continue;
        }
        let (width, height) = (window.width()?, window.height()?);
        if width < MIN_WINDOW_SIZE || height < MIN_WINDOW_SIZE {
            continue;
        }
        windows.push(WindowInfo {
            id: window.id()?,
            x: f64::from(window.x()?),
            y: f64::from(window.y()?),
            width: f64::from(width),
            height: f64::from(height),
        });
    }
    Ok(windows)
}

fn open_overlay(app: &AppHandle, index: usize, x: i32, y: i32, width: u32, height: u32) -> tauri::Result<()> {
    let window = WebviewWindowBuilder::new(app, format!("{WINDOW_PREFIX}{index}"), WebviewUrl::App(PAGE.into()))
        .title("Snapclone")
        .position(f64::from(x), f64::from(y))
        .inner_size(f64::from(width), f64::from(height))
        .decorations(false)
        .resizable(false)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible_on_all_workspaces(true)
        .accept_first_mouse(true)
        .focused(index == 0)
        .build()?;
    raise_above_menu_bar(app, window);
    Ok(())
}

/// "Always on top" is the floating level, which the menu bar and the Dock still cover.
#[cfg(target_os = "macos")]
fn raise_above_menu_bar(app: &AppHandle, window: WebviewWindow) {
    let result = app.run_on_main_thread(move || {
        use objc2_app_kit::{NSPopUpMenuWindowLevel, NSWindow};
        let Ok(pointer) = window.ns_window() else { return };
        // SAFETY: Tauri hands out the live NSWindow of this window, and AppKit is only
        // touched here on the main thread.
        let ns_window = unsafe { &*pointer.cast::<NSWindow>() };
        ns_window.setLevel(NSPopUpMenuWindowLevel);
    });
    if let Err(error) = result {
        eprintln!("selector window level could not be raised: {error}");
    }
}

#[cfg(not(target_os = "macos"))]
fn raise_above_menu_bar(_app: &AppHandle, _window: WebviewWindow) {}

fn close_overlays(app: &AppHandle) {
    for (label, window) in app.webview_windows() {
        if label.starts_with(WINDOW_PREFIX) {
            let _ = window.destroy();
        }
    }
}

/// Ends the selection: closes every overlay and frees the frozen images. Returns the
/// session so a finished selection can still crop from it.
fn end(app: &AppHandle) -> Option<Session> {
    let session = app.state::<Selection>().0.lock().unwrap().take();
    if session.is_some() {
        close_overlays(app);
        capture::selection_ended(app);
    }
    session
}

/// An overlay closed some other way (Cmd+W, a crash) must not leave the others behind.
pub fn on_window_destroyed(app: &AppHandle, label: &str) {
    if label.starts_with(WINDOW_PREFIX) {
        end(app);
    }
}

/// The monitor an overlay belongs to comes from its label, never from the page.
fn monitor_index(window: &WebviewWindow) -> Result<usize, String> {
    window
        .label()
        .strip_prefix(WINDOW_PREFIX)
        .and_then(|index| index.parse().ok())
        .ok_or_else(|| "not a selector window".to_string())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayInfo {
    pixel_width: u32,
    pixel_height: u32,
    /// Windows on this monitor in points relative to its top-left corner, front-most first.
    windows: Vec<WindowInfo>,
}

#[tauri::command]
pub fn selector_info(window: WebviewWindow, selection: State<'_, Selection>) -> Result<OverlayInfo, String> {
    let index = monitor_index(&window)?;
    let session = selection.0.lock().unwrap();
    let session = session.as_ref().ok_or("no selection in progress")?;
    let shot = session.monitors.get(index).ok_or("unknown monitor")?;
    let windows = session
        .windows
        .iter()
        .map(|info| WindowInfo { x: info.x - f64::from(shot.x), y: info.y - f64::from(shot.y), ..info.clone() })
        .filter(|info| {
            info.x < f64::from(shot.width) && info.y < f64::from(shot.height) && info.x + info.width > 0.0 && info.y + info.height > 0.0
        })
        .collect();
    Ok(OverlayInfo { pixel_width: shot.image.width(), pixel_height: shot.image.height(), windows })
}

/// The frozen monitor image as raw RGBA, so nothing has to be encoded before showing it.
#[tauri::command]
pub fn selector_frame(window: WebviewWindow, selection: State<'_, Selection>) -> Result<Response, String> {
    let index = monitor_index(&window)?;
    let session = selection.0.lock().unwrap();
    let shot = session.as_ref().and_then(|session| session.monitors.get(index)).ok_or("no selection in progress")?;
    Ok(Response::new(shot.image.as_raw().clone()))
}

#[tauri::command]
pub fn cancel_selection(app: AppHandle) {
    end(&app);
}

/// Crops the dragged rectangle (points, relative to the overlay) from its monitor image.
#[tauri::command]
pub async fn finish_selection(app: AppHandle, window: WebviewWindow, rect: Rect) -> Result<(), String> {
    let index = monitor_index(&window)?;
    let session = end(&app).ok_or("no selection in progress")?;
    let shot = session.monitors.into_iter().nth(index).ok_or("unknown monitor")?;
    let scale = shot.scale();
    let (x, y, width, height) =
        crop_rect(rect, scale, shot.image.width(), shot.image.height()).ok_or("the selection is empty")?;
    let image = xcap::image::imageops::crop_imm(&shot.image, x, y, width, height).to_image();
    store(&app, image, scale).await
}

/// Captures one window by itself (so overlapping windows are left out). If the system
/// refuses, the window's area is cropped from the frozen monitor image instead.
#[tauri::command]
pub async fn finish_window(app: AppHandle, window: WebviewWindow, id: u32) -> Result<(), String> {
    let index = monitor_index(&window)?;
    let session = end(&app).ok_or("no selection in progress")?;
    let info = session.windows.iter().find(|info| info.id == id).cloned().ok_or("unknown window")?;
    let shot = session.monitors.into_iter().nth(index).ok_or("unknown monitor")?;

    let captured = tauri::async_runtime::spawn_blocking(move || capture_window(id))
        .await
        .map_err(|error| error.to_string())?;
    let (image, scale) = match captured {
        Ok(image) => {
            let scale = f64::from(image.width()) / info.width.max(1.0);
            (image, scale)
        }
        Err(error) => {
            eprintln!("window capture failed, cropping the screen instead: {error}");
            let local = Rect { x: info.x - f64::from(shot.x), y: info.y - f64::from(shot.y), width: info.width, height: info.height };
            let scale = shot.scale();
            let (x, y, width, height) =
                crop_rect(local, scale, shot.image.width(), shot.image.height()).ok_or("the window is off screen")?;
            (xcap::image::imageops::crop_imm(&shot.image, x, y, width, height).to_image(), scale)
        }
    };
    store(&app, image, scale).await
}

fn capture_window(id: u32) -> Result<RgbaImage, String> {
    let windows = xcap::Window::all().map_err(|error| error.to_string())?;
    let window = windows
        .into_iter()
        .find(|window| window.id().is_ok_and(|window_id| window_id == id))
        .ok_or("the window is gone")?;
    window.capture_image().map_err(|error| error.to_string())
}

/// Encodes off the async runtime and hands the file to the normal capture flow.
async fn store(app: &AppHandle, image: RgbaImage, scale: f64) -> Result<(), String> {
    let dir = capture::capture_dir(app)?;
    let id = uuid::Uuid::new_v4().to_string();
    let path = dir.join(format!("{id}.png"));
    let target = path.clone();
    tauri::async_runtime::spawn_blocking(move || write_png(&target, &image))
        .await
        .map_err(|error| error.to_string())??;
    capture::deliver(app, id, path, scale);
    Ok(())
}

/// Fast compression: captures are re-encoded on export anyway, so speed matters more here.
fn write_png(path: &Path, image: &RgbaImage) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|error| error.to_string())?;
    png::PngEncoder::new_with_quality(std::io::BufWriter::new(file), png::CompressionType::Fast, png::FilterType::Adaptive)
        .write_image(image.as_raw(), image.width(), image.height(), xcap::image::ExtendedColorType::Rgba8)
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::{Rect, crop_rect};

    fn rect(x: f64, y: f64, width: f64, height: f64) -> Rect {
        Rect { x, y, width, height }
    }

    #[test]
    fn one_to_one_monitor_keeps_points_as_pixels() {
        assert_eq!(crop_rect(rect(10.0, 20.0, 300.0, 200.0), 1.0, 2560, 1440), Some((10, 20, 300, 200)));
    }

    #[test]
    fn retina_monitor_doubles_and_rounds_outwards() {
        assert_eq!(crop_rect(rect(10.0, 20.0, 300.0, 200.0), 2.0, 5120, 2880), Some((20, 40, 600, 400)));
        assert_eq!(crop_rect(rect(0.4, 0.4, 1.2, 1.2), 2.0, 100, 100), Some((0, 0, 4, 4)));
    }

    #[test]
    fn drags_up_and_left_are_normalized() {
        assert_eq!(crop_rect(rect(310.0, 220.0, -300.0, -200.0), 1.0, 2560, 1440), Some((10, 20, 300, 200)));
    }

    #[test]
    fn selection_is_clamped_to_the_image() {
        assert_eq!(crop_rect(rect(-50.0, 1400.0, 100.0, 100.0), 1.0, 1920, 1440), Some((0, 1400, 50, 40)));
    }

    #[test]
    fn empty_or_invalid_selections_are_rejected() {
        assert_eq!(crop_rect(rect(10.0, 10.0, 0.0, 50.0), 1.0, 100, 100), None);
        assert_eq!(crop_rect(rect(200.0, 10.0, 50.0, 50.0), 1.0, 100, 100), None);
        assert_eq!(crop_rect(rect(f64::NAN, 10.0, 50.0, 50.0), 1.0, 100, 100), None);
        assert_eq!(crop_rect(rect(10.0, 10.0, 50.0, 50.0), 0.0, 100, 100), None);
    }
}
