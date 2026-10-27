use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    AppHandle,
};

use crate::capture;

pub fn build(app: &AppHandle) -> tauri::Result<()> {
    let capture_item = MenuItem::with_id(app, "capture", "Capturar región", true, Some("Cmd+Shift+2"))?;
    let quit_item = MenuItem::with_id(app, "quit", "Salir", true, Some("Cmd+Q"))?;
    let menu = Menu::with_items(app, &[&capture_item, &quit_item])?;

    let mut tray = TrayIconBuilder::with_id("main")
        .tooltip("Snapclone")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "capture" => capture::start(app.clone()),
            "quit" => app.exit(0),
            _ => {}
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}
