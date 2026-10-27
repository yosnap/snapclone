mod capture;
mod export;
mod history;
mod redact;
mod tray;

use tauri::{Manager, WindowEvent};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let capture_shortcut = Shortcut::new(Some(Modifiers::SUPER | Modifiers::SHIFT), Code::Digit2);

    tauri::Builder::default()
        .plugin(tauri_plugin_store::Builder::new().build())
        .manage(capture::Captures::default())
        .manage(export::PendingSave::default())
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(move |app, shortcut, event| {
                    if event.state() == ShortcutState::Pressed && shortcut == &capture_shortcut {
                        capture::start(app.clone());
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
        // Menu bar app: closing the editor hides it instead of quitting.
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event
                && window.label() == capture::EDITOR_WINDOW
            {
                api.prevent_close();
                let _ = window.hide();
            }
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
            history::delete_history
        ])
        .setup(move |app| {
            app.manage(history::History::open(app.handle()));
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            tray::build(app.handle())?;
            app.global_shortcut().register(capture_shortcut)?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
