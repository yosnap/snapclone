//! Full-screen capture of the monitor under the pointer, taken straight away without the
//! selector.

use xcap::image::RgbaImage;

use crate::capture;

/// The image and its pixels per point, taken before anything shows or focuses a window (see
/// the invariant in `capture::run_capture`). Errors make the caller fall back to `screencapture`.
pub fn capture() -> Result<(RgbaImage, f64), String> {
    if !capture::screen_capture_allowed() {
        return Err("Screen Recording permission is not granted".into());
    }
    let monitor = match pointer_location() {
        Some((x, y)) => xcap::Monitor::from_point(x, y).map_err(|error| error.to_string())?,
        None => primary_monitor()?,
    };
    let image = monitor.capture_image().map_err(|error| error.to_string())?;
    let scale = monitor_scale(&monitor, image.width())?;
    Ok((image, scale))
}

/// Image pixels per CSS point for `monitor`, whose capture is already `image_width` wide. On
/// macOS, `width()` is already in points, so the ratio against the image's pixel width gives
/// the Retina factor directly. On Windows, `width()` is physical pixels (the same unit the
/// capture is already in), so that ratio is always 1 — the real DPI factor comes from
/// `scale_factor()` instead.
#[cfg(target_os = "macos")]
fn monitor_scale(monitor: &xcap::Monitor, image_width: u32) -> Result<f64, String> {
    let width = monitor.width().map_err(|error| error.to_string())?;
    Ok(f64::from(image_width) / f64::from(width.max(1)))
}

#[cfg(not(target_os = "macos"))]
fn monitor_scale(monitor: &xcap::Monitor, _image_width: u32) -> Result<f64, String> {
    monitor.scale_factor().map(f64::from).map_err(|error| error.to_string())
}

fn primary_monitor() -> Result<xcap::Monitor, String> {
    let monitors = xcap::Monitor::all().map_err(|error| error.to_string())?;
    let primary = monitors.iter().position(|monitor| monitor.is_primary().unwrap_or(false));
    monitors.into_iter().nth(primary.unwrap_or(0)).ok_or_else(|| "no monitors found".to_string())
}

/// The pointer in global points with the origin at the top left of the main display, the
/// same space `xcap` uses for monitors.
#[cfg(target_os = "macos")]
pub fn pointer_location() -> Option<(i32, i32)> {
    use std::ffi::c_void;

    use objc2_core_foundation::CGPoint;

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGEventCreate(source: *const c_void) -> *mut c_void;
        fn CGEventGetLocation(event: *mut c_void) -> CGPoint;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(object: *const c_void);
    }
    // SAFETY: an event created without a source only snapshots the current pointer state; it
    // is checked for null, read once and released.
    unsafe {
        let event = CGEventCreate(std::ptr::null());
        if event.is_null() {
            return None;
        }
        let point = CGEventGetLocation(event);
        CFRelease(event);
        Some((point.x.floor() as i32, point.y.floor() as i32))
    }
}

/// The pointer in global physical pixels (the same unit `xcap::Monitor` coordinates use on
/// Windows — see the Phase 1 HiDPI note in the port plan for why this must never be converted
/// to a per-monitor "points" space before a monitor has actually been resolved for it).
#[cfg(windows)]
pub fn pointer_location() -> Option<(i32, i32)> {
    use windows::Win32::{Foundation::POINT, UI::WindowsAndMessaging::GetCursorPos};

    let mut point = POINT::default();
    // SAFETY: `point` is a valid, owned out-parameter for the duration of the call.
    unsafe { GetCursorPos(&mut point) }.ok().map(|()| (point.x, point.y))
}

#[cfg(not(any(target_os = "macos", windows)))]
pub fn pointer_location() -> Option<(i32, i32)> {
    None
}
