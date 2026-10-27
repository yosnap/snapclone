use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    AppHandle,
};

use crate::{capture, editor_window};

pub fn build(app: &AppHandle) -> tauri::Result<()> {
    let capture_item = MenuItem::with_id(app, "capture", "Capturar región", true, Some("Cmd+Shift+2"))?;
    let open_item = MenuItem::with_id(app, "open", "Abrir Snapclone", true, None::<&str>)?;
    let settings_item = MenuItem::with_id(app, SETTINGS_ID, "Ajustes…", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "Salir", true, Some("Cmd+Q"))?;
    let separator = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&capture_item, &separator, &open_item, &settings_item, &separator, &quit_item])?;

    let mut tray = TrayIconBuilder::with_id("main")
        .tooltip("Snapclone")
        .menu(&menu)
        .show_menu_on_left_click(true);
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

/// Shared by the menu bar icon and the app menu, so both open the same settings.
pub const SETTINGS_ID: &str = "settings";

/// Menu event listeners are global in Tauri: this one handles the tray and the app menu.
pub fn on_menu_event(app: &AppHandle, id: &str) {
    match id {
        "capture" => capture::start(app.clone()),
        "open" => editor_window::show(app, false),
        SETTINGS_ID => editor_window::show_settings(app),
        "quit" => app.exit(0),
        _ => {}
    }
}

/// Adds "Ajustes…" (⌘,) to the default app menu, after "About".
pub fn add_settings_to_app_menu(app: &AppHandle) -> tauri::Result<()> {
    use tauri::menu::MenuItemKind;
    let Some(menu) = app.menu() else { return Ok(()) };
    let Some(MenuItemKind::Submenu(app_menu)) = menu.items()?.into_iter().next() else { return Ok(()) };
    let settings = MenuItem::with_id(app, SETTINGS_ID, "Ajustes…", true, Some("Cmd+,"))?;
    app_menu.insert(&settings, 2)?;
    app_menu.insert(&PredefinedMenuItem::separator(app)?, 3)?;
    Ok(())
}
