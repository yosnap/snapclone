//! The editor window and the Dock icon. The window starts hidden and is shown centred on the
//! main display, so it never opens behind other apps or on a monitor that is gone.

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewWindow};

use crate::{
    capture::EDITOR_WINDOW,
    geometry::{self, Rect},
    history, prefs,
};

const DOCK_HIDDEN_KEY: &str = "hideDockIcon";
const SETTINGS_EVENT: &str = "open-settings";
const NOTICE_EVENT: &str = "editor-notice";
const REOPEN_EVENT: &str = "reopen-capture";

#[derive(Clone, Serialize)]
struct Notice<'a> {
    message: &'a str,
    error: bool,
}

/// Shows, raises and focuses the editor. At launch, or when it would sit off every monitor,
/// it is first centred on the main display.
pub fn show(app: &AppHandle, recenter: bool) {
    let Some(window) = app.get_webview_window(EDITOR_WINDOW) else { return };
    if recenter || !is_reachable(app, &window) {
        center_on_primary(app, &window);
    }
    let _ = window.unminimize();
    if let Err(error) = window.show() {
        eprintln!("editor window could not be shown: {error}");
    }
    let _ = window.set_focus();
}

/// Hides the editor; the app keeps running in the menu bar and the Dock.
pub fn hide(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(EDITOR_WINDOW) {
        let _ = window.hide();
    }
}

/// Whether the editor is on screen. A hidden app (⌘H or "Ocultar Snapclone") counts as not.
pub fn is_visible(app: &AppHandle) -> bool {
    app.get_webview_window(EDITOR_WINDOW).is_some_and(|window| window.is_visible().unwrap_or(false))
}

/// After "Ocultar Snapclone" the whole app is hidden, and so would be every overlay a
/// capture opens. This unhides it without activating it, which keeps the capture invariant
/// (nothing activates Snapclone before the shot); queued on the main thread after any hide
/// requested before it, so a hidden editor stays out of the shot.
pub fn unhide_without_activation(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    if let Err(error) = app.run_on_main_thread(|| {
        if let Some(main_thread) = objc2::MainThreadMarker::new() {
            objc2_app_kit::NSApplication::sharedApplication(main_thread).unhideWithoutActivation();
        }
    }) {
        eprintln!("the app could not be unhidden: {error}");
    }
}

/// "Ocultar Snapclone" (⌘Q): closes every window and gives the focus back to the previous
/// app. The process keeps running in the menu bar with its global shortcuts; the Dock icon,
/// the menu bar or a capture bring the editor back.
pub fn hide_app(app: &AppHandle) {
    for (label, window) in app.webview_windows() {
        // Selector and scroll windows clean up their session when they close.
        if label != EDITOR_WINDOW
            && let Err(error) = window.close()
        {
            eprintln!("window {label} could not be closed: {error}");
        }
    }
    hide(app);
    #[cfg(target_os = "macos")]
    if let Err(error) = app.hide() {
        eprintln!("the app could not be hidden: {error}");
    }
}

/// Opens the newest capture of the history with its document, in the editor.
pub fn reopen_last(app: &AppHandle) {
    match history::latest_id(app) {
        Ok(Some(id)) => {
            show(app, false);
            if let Err(error) = app.emit_to(EDITOR_WINDOW, REOPEN_EVENT, id) {
                eprintln!("reopen-capture emit failed: {error}");
            }
        }
        Ok(None) => notify(app, "Todavía no hay ninguna captura que reabrir", false),
        Err(error) => notify(app, &format!("No se pudo leer el historial: {error}"), true),
    }
}

#[tauri::command]
pub fn reopen_last_capture(app: AppHandle) {
    reopen_last(&app);
}

/// Brings the editor forward with its settings open.
pub fn show_settings(app: &AppHandle) {
    show(app, false);
    if let Err(error) = app.emit_to(EDITOR_WINDOW, SETTINGS_EVENT, ()) {
        eprintln!("open-settings emit failed: {error}");
    }
}

/// Brings the editor forward with a message in its status line, so the outcome of an action
/// started from the menu bar is never lost behind a hidden window.
pub fn notify(app: &AppHandle, message: &str, error: bool) {
    if error {
        eprintln!("{message}");
    }
    show(app, false);
    emit_notice(app, message, error);
}

/// Puts a message in the editor's status line without bringing it forward.
pub fn status(app: &AppHandle, message: &str) {
    emit_notice(app, message, false);
}

fn emit_notice(app: &AppHandle, message: &str, error: bool) {
    if let Err(emit_error) = app.emit_to(EDITOR_WINDOW, NOTICE_EVENT, Notice { message, error }) {
        eprintln!("editor-notice emit failed: {emit_error}");
    }
}

fn to_rect(position: PhysicalPosition<i32>, size: PhysicalSize<u32>) -> Rect {
    Rect::new(position.x, position.y, size.width, size.height)
}

fn is_reachable(app: &AppHandle, window: &WebviewWindow) -> bool {
    let (Ok(position), Ok(size), Ok(monitors)) = (window.outer_position(), window.outer_size(), app.available_monitors())
    else {
        return false;
    };
    let monitors: Vec<Rect> = monitors.iter().map(|monitor| to_rect(*monitor.position(), *monitor.size())).collect();
    geometry::is_reachable(to_rect(position, size), &monitors)
}

fn center_on_primary(app: &AppHandle, window: &WebviewWindow) {
    let primary = app.primary_monitor().ok().flatten().or_else(|| app.available_monitors().ok()?.into_iter().next());
    let (Some(monitor), Ok(size)) = (primary, window.outer_size()) else {
        eprintln!("no monitor to centre the editor on");
        return;
    };
    let work_area = monitor.work_area();
    let area = to_rect(work_area.position, work_area.size);
    let target = geometry::centered(size.width, size.height, area);
    if (target.width, target.height) != (size.width, size.height) {
        let _ = window.set_size(PhysicalSize::new(target.width, target.height));
    }
    if let Err(error) = window.set_position(PhysicalPosition::new(target.x, target.y)) {
        eprintln!("editor window could not be centred: {error}");
    }
}

/// Saved choice; the Dock icon is shown unless the user hid it.
pub fn dock_hidden(app: &AppHandle) -> bool {
    prefs::read(app, DOCK_HIDDEN_KEY).unwrap_or(false)
}

#[cfg(target_os = "macos")]
pub fn activation_policy(hidden: bool) -> tauri::ActivationPolicy {
    if hidden { tauri::ActivationPolicy::Accessory } else { tauri::ActivationPolicy::Regular }
}

#[tauri::command]
pub fn get_dock_hidden(app: AppHandle) -> bool {
    dock_hidden(&app)
}

/// Saves and applies the choice at once. Leaving the Dock can deactivate the app, so the
/// editor is brought back to the front afterwards.
#[tauri::command]
pub fn set_dock_hidden(app: AppHandle, hidden: bool) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    app.set_activation_policy(activation_policy(hidden)).map_err(|error| error.to_string())?;
    prefs::write(&app, DOCK_HIDDEN_KEY, hidden)?;
    show(&app, false);
    Ok(())
}
