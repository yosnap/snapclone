//! App-wide preferences shown in the Preferences window. They live in `settings.json` next to
//! the editor's own keys, but Rust owns them: it validates every change and applies it.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::{capture, editor_window, output, prefs};

pub const SETTINGS_WINDOW: &str = "settings";
const SOUND_KEY: &str = "soundOn";
const CLOSE_AFTER_COPY_KEY: &str = "closeAfterCopy";
const HIDE_WINDOWS_KEY: &str = "hideWindowsOnCapture";
const HISTORY_EVENT: &str = "open-history";
const SCREEN_CAPTURE_SOUND: &str =
    "/System/Library/Components/CoreAudio.component/Contents/SharedSupport/SystemSounds/system/Screen Capture.aif";

/// Everything the Preferences window shows, read in one call.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    sound_on: bool,
    close_after_copy: bool,
    hide_windows_on_capture: bool,
    hide_dock_icon: bool,
    default_file_name: String,
    save_folder: Option<String>,
    history_folder: Option<String>,
    version: String,
}

/// The switches the Preferences window can flip. The Dock icon has its own command because
/// changing it also changes the activation policy.
#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "camelCase")]
pub enum Toggle {
    SoundOn,
    CloseAfterCopy,
    HideWindowsOnCapture,
}

impl Toggle {
    fn key(self) -> &'static str {
        match self {
            Self::SoundOn => SOUND_KEY,
            Self::CloseAfterCopy => CLOSE_AFTER_COPY_KEY,
            Self::HideWindowsOnCapture => HIDE_WINDOWS_KEY,
        }
    }

    /// Defaults keep the behaviour Snapclone had before these were configurable.
    fn default_value(self) -> bool {
        matches!(self, Self::HideWindowsOnCapture)
    }
}

pub fn toggle(app: &AppHandle, toggle: Toggle) -> bool {
    prefs::read(app, toggle.key()).unwrap_or_else(|| toggle.default_value())
}

#[tauri::command]
pub fn get_app_settings(app: AppHandle) -> AppSettings {
    AppSettings {
        sound_on: toggle(&app, Toggle::SoundOn),
        close_after_copy: toggle(&app, Toggle::CloseAfterCopy),
        hide_windows_on_capture: toggle(&app, Toggle::HideWindowsOnCapture),
        hide_dock_icon: editor_window::dock_hidden(&app),
        default_file_name: output::file_name_pattern(&app),
        save_folder: output::save_folder(&app).map(|path| path.to_string_lossy().into_owned()),
        history_folder: capture::capture_dir(&app).ok().map(|path| path.to_string_lossy().into_owned()),
        version: app.package_info().version.to_string(),
    }
}

#[tauri::command]
pub fn set_app_toggle(app: AppHandle, toggle: Toggle, value: bool) -> Result<(), String> {
    prefs::write(&app, toggle.key(), value)
}

/// Opens the Preferences window, or brings it forward when it is already open.
pub fn open(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(SETTINGS_WINDOW) {
        let _ = window.unminimize();
        if let Err(error) = window.show() {
            eprintln!("settings window could not be shown: {error}");
        }
        let _ = window.set_focus();
        return;
    }
    let built = WebviewWindowBuilder::new(app, SETTINGS_WINDOW, WebviewUrl::App(PathBuf::from("settings.html")))
        .title("Ajustes de Snapclone")
        .inner_size(440.0, 800.0)
        .resizable(false)
        .maximizable(false)
        .center()
        .build();
    if let Err(error) = built {
        editor_window::notify(app, &format!("No se pudieron abrir los ajustes: {error}"), true);
    }
}

#[tauri::command]
pub fn open_settings(app: AppHandle) {
    open(&app);
}

pub fn is_window_visible(app: &AppHandle) -> bool {
    app.get_webview_window(SETTINGS_WINDOW).is_some_and(|window| window.is_visible().unwrap_or(false))
}

/// Hides the Preferences window during a capture, like the editor.
pub fn hide_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(SETTINGS_WINDOW) {
        let _ = window.hide();
    }
}

/// Brings forward the window the user is working in after a change that may deactivate the
/// app: the Preferences window when open, the editor otherwise.
pub fn refocus(app: &AppHandle) {
    match app.get_webview_window(SETTINGS_WINDOW) {
        Some(window) => {
            let _ = window.show();
            let _ = window.set_focus();
        }
        None => editor_window::show(app, false),
    }
}

/// "Ver historial" in the Preferences window: the history lives in the editor.
#[tauri::command]
pub fn show_history(app: AppHandle) {
    editor_window::show(&app, false);
    if let Err(error) = app.emit_to(capture::EDITOR_WINDOW, HISTORY_EVENT, ()) {
        eprintln!("open-history emit failed: {error}");
    }
}

#[tauri::command]
pub fn open_history_folder(app: AppHandle) -> Result<(), String> {
    output::open_in_finder(&capture::capture_dir(&app)?, false)
}

/// The system screenshot sound, played without waiting for it when the user enabled it.
pub fn play_capture_sound(app: &AppHandle) {
    if !toggle(app, Toggle::SoundOn) {
        return;
    }
    tauri::async_runtime::spawn(async {
        match tokio::process::Command::new("/usr/bin/afplay").arg(SCREEN_CAPTURE_SOUND).status().await {
            Ok(status) if !status.success() => eprintln!("capture sound failed: {status}"),
            Err(error) => eprintln!("capture sound could not be played: {error}"),
            Ok(_) => {}
        }
    });
}
