mod capture;
mod editor_window;
mod export;
mod geometry;
mod history;
mod images;
mod prefs;
mod redact;
mod screen;
mod selector;
mod shortcuts;
mod tray;

use tauri::{Manager, RunEvent, WindowEvent};
use tauri_plugin_global_shortcut::ShortcutState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_store::Builder::new().build())
        .manage(capture::Captures::default())
        .manage(export::PendingSave::default())
        .manage(selector::Selection::default())
        .manage(shortcuts::Shortcuts::default())
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        shortcuts::on_shortcut(app, shortcut);
                    }
                })
                .build(),
        )
        .register_asynchronous_uri_scheme_protocol("capture", |ctx, request, responder| {
            let app = ctx.app_handle().clone();
            tauri::async_runtime::spawn_blocking(move || {
                responder.respond(capture::serve(&app, &request));
            });
        })
        .on_menu_event(|app, event| tray::on_menu_event(app, event.id.as_ref()))
        // Closing the editor hides it; the app keeps running in the menu bar and the Dock.
        .on_window_event(|window, event| match event {
            WindowEvent::CloseRequested { api, .. } if window.label() == capture::EDITOR_WINDOW => {
                api.prevent_close();
                let _ = window.hide();
            }
            // A shortcut recorder left open never keeps the global shortcuts paused.
            WindowEvent::Focused(false) if window.label() == capture::EDITOR_WINDOW => shortcuts::resume(window.app_handle()),
            WindowEvent::Destroyed => selector::on_window_destroyed(window.app_handle(), window.label()),
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            capture::start_capture,
            export::copy_png,
            export::choose_save_path,
            export::save_image,
            redact::detect_sensitive,
            history::list_history,
            history::save_document,
            history::open_history,
            history::delete_history,
            selector::selector_info,
            selector::selector_frame,
            selector::cancel_selection,
            selector::finish_selection,
            selector::finish_window,
            editor_window::get_dock_hidden,
            editor_window::set_dock_hidden,
            shortcuts::get_shortcuts,
            shortcuts::set_shortcut,
            shortcuts::pause_shortcuts
        ])
        .setup(|app| {
            app.manage(history::History::open(app.handle()));
            #[cfg(target_os = "macos")]
            app.set_activation_policy(editor_window::activation_policy(editor_window::dock_hidden(app.handle())));
            let registered = shortcuts::init(app.handle());
            tray::build(app.handle(), &registered)?;
            tray::add_settings_to_app_menu(app.handle())?;
            editor_window::show(app.handle(), true);
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            // A click on the Dock icon, or opening the app again while it runs.
            if let RunEvent::Reopen { .. } = event {
                editor_window::show(app, false);
            }
        });
}
