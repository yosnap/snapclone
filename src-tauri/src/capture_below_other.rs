//! Non-macOS placeholder for `capture_below`, selected via `#[path]` in `lib.rs`. Phase 2 fills
//! in a real Windows implementation behind `#[cfg(windows)]` without changing this signature.

use xcap::image::RgbaImage;

use crate::geometry::Rect;

/// `rect` is in global physical pixels (see the Phase 1 HiDPI note). Windows has no capture-time
/// "exclude these windows" flag like macOS's `kCGWindowListOptionOnScreenBelowWindow` — `below`
/// is unused here. The scroll capture's own frame/toolbar windows sit just OUTSIDE the zone by
/// construction (`scroll.rs::begin`'s `EDGE`/`geometry::toolbar_position`), so a plain region
/// capture already leaves them out in the common case; see the Windows port plan's Phase 2 spike
/// for the one case (a zone pinned to a monitor edge) where that stops being true.
#[cfg(windows)]
pub fn capture(rect: Rect, _below: u32) -> Result<RgbaImage, String> {
    let monitor = xcap::Monitor::from_point(rect.x, rect.y).map_err(|error| error.to_string())?;
    let origin_x = monitor.x().map_err(|error| error.to_string())?;
    let origin_y = monitor.y().map_err(|error| error.to_string())?;
    let local_x = u32::try_from(rect.x - origin_x).map_err(|_| "the zone is off its monitor".to_string())?;
    let local_y = u32::try_from(rect.y - origin_y).map_err(|_| "the zone is off its monitor".to_string())?;
    monitor.capture_region(local_x, local_y, rect.width, rect.height).map_err(|error| error.to_string())
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn capture(_rect: Rect, _below: u32) -> Result<RgbaImage, String> {
    Err("scroll capture is only available on macOS and Windows".into())
}
