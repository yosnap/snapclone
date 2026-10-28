//! The "Copiada al portapapeles" bezel, like the macOS volume one: a small dark square on
//! the monitor under the pointer that fades out by itself. It never takes the focus nor
//! activates the app, and lets every click through to the windows below.

use std::{
    sync::atomic::{AtomicU32, Ordering},
    time::Duration,
};

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::{
    geometry::{self, Rect},
    screen,
};

const WINDOW_PREFIX: &str = "hud-";
const PAGE: &str = "hud.html";
/// Side of the bezel, in points.
const SIZE: u32 = 180;
/// The page fades in and out within 1.2 s of loading; the window outlives that a little so
/// the fade is never cut short while the page is still loading.
const LIFETIME: Duration = Duration::from_millis(1600);

/// Each bezel gets its own label: one shown again before the last is destroyed would
/// otherwise clash with it.
static NEXT_ID: AtomicU32 = AtomicU32::new(0);

/// Shows the bezel, replacing one still on screen, and destroys it after `LIFETIME`.
pub fn show_copied(app: &AppHandle) {
    close_all(app);
    let label = format!("{WINDOW_PREFIX}{}", NEXT_ID.fetch_add(1, Ordering::Relaxed));
    let window = match open(app, &label) {
        Ok(window) => window,
        Err(error) => {
            // The copy itself worked; only the confirmation is missing.
            eprintln!("copied bezel could not be shown: {error}");
            return;
        }
    };
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(LIFETIME).await;
        let _ = window.destroy();
    });
}

fn open(app: &AppHandle, label: &str) -> Result<WebviewWindow, String> {
    let builder = WebviewWindowBuilder::new(app, label, WebviewUrl::App(PAGE.into()))
        .title("Snapclone")
        .inner_size(f64::from(SIZE), f64::from(SIZE))
        .decorations(false)
        .resizable(false)
        .shadow(false)
        // Only the page's rounded square shows, and nothing flashes while it loads.
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible_on_all_workspaces(true)
        .focused(false)
        .focusable(false);
    let builder = match placement(app) {
        Some(spot) => builder.position(f64::from(spot.x), f64::from(spot.y)),
        None => builder.center(),
    };
    let window = builder.build().map_err(|error| error.to_string())?;
    window.set_ignore_cursor_events(true).map_err(|error| error.to_string())?;
    raise(app, window.clone());
    Ok(window)
}

/// The spot in the lower third of the monitor under the pointer, in global points like the
/// pointer. `None` when the pointer or the monitors cannot be read.
fn placement(app: &AppHandle) -> Option<Rect> {
    let (x, y) = screen::pointer_location()?;
    let areas: Vec<(Rect, Rect)> = app
        .available_monitors()
        .ok()?
        .iter()
        .map(|monitor| {
            let scale = monitor.scale_factor();
            let points = |x: i32, y: i32, width: u32, height: u32| {
                let to_point = |value: f64| (value / scale).round();
                Rect::new(
                    to_point(f64::from(x)) as i32,
                    to_point(f64::from(y)) as i32,
                    to_point(f64::from(width)) as u32,
                    to_point(f64::from(height)) as u32,
                )
            };
            let (position, size) = (monitor.position(), monitor.size());
            let work = monitor.work_area();
            (
                points(position.x, position.y, size.width, size.height),
                points(work.position.x, work.position.y, work.size.width, work.size.height),
            )
        })
        .collect();
    let contains = |area: &Rect| {
        x >= area.x
            && y >= area.y
            && i64::from(x) < i64::from(area.x) + i64::from(area.width)
            && i64::from(y) < i64::from(area.y) + i64::from(area.height)
    };
    let (_, work) = areas.iter().find(|(full, _)| contains(full)).or_else(|| areas.first())?;
    Some(geometry::bezel(*work, SIZE))
}

/// "Always on top" is the floating level, which the menu bar, the Dock and full-screen
/// overlays still cover; the bezel goes above them like the system ones.
#[cfg(target_os = "macos")]
fn raise(app: &AppHandle, window: WebviewWindow) {
    let result = app.run_on_main_thread(move || {
        use objc2_app_kit::{NSPopUpMenuWindowLevel, NSWindow};
        let Ok(pointer) = window.ns_window() else { return };
        // SAFETY: Tauri hands out the live NSWindow of this window, and AppKit is only
        // touched here on the main thread.
        let ns_window = unsafe { &*pointer.cast::<NSWindow>() };
        ns_window.setLevel(NSPopUpMenuWindowLevel);
    });
    if let Err(error) = result {
        eprintln!("bezel window level could not be raised: {error}");
    }
}

#[cfg(not(target_os = "macos"))]
fn raise(_app: &AppHandle, _window: WebviewWindow) {}

fn close_all(app: &AppHandle) {
    for (label, window) in app.webview_windows() {
        if label.starts_with(WINDOW_PREFIX) {
            let _ = window.destroy();
        }
    }
}
