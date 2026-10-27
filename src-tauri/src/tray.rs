use tauri::{
    menu::{Menu, MenuItem, MenuItemKind, PredefinedMenuItem},
    tray::TrayIconBuilder,
    AppHandle, Manager, Wry,
};

use crate::{
    capture::{self, CaptureKind},
    editor_window,
    shortcuts::ShortcutEntry,
};

/// Shared by the menu bar icon and the app menu, so both open the same settings.
const SETTINGS_ID: &str = "settings";
const CAPTURE_PREFIX: &str = "capture-";

/// The capture entries, kept so their shortcuts can be updated when they change.
struct CaptureItems(Vec<(CaptureKind, MenuItem<Wry>)>);

fn capture_title(kind: CaptureKind) -> &'static str {
    match kind {
        CaptureKind::Region => "Capturar zona",
        CaptureKind::Window => "Capturar ventana",
        CaptureKind::Screen => "Capturar pantalla completa",
        CaptureKind::Scroll => "Capturar con scroll",
    }
}

pub fn build(app: &AppHandle, shortcuts: &[ShortcutEntry]) -> tauri::Result<()> {
    let captures = CaptureKind::ALL
        .into_iter()
        .map(|kind| {
            let id = format!("{CAPTURE_PREFIX}{}", kind.id());
            MenuItem::with_id(app, id, capture_title(kind), true, None::<&str>).map(|item| (kind, item))
        })
        .collect::<tauri::Result<Vec<_>>>()?;
    let open_item = MenuItem::with_id(app, "open", "Abrir Snapclone", true, None::<&str>)?;
    let settings_item = MenuItem::with_id(app, SETTINGS_ID, "Ajustes…", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "Salir", true, Some("Cmd+Q"))?;
    let separator = PredefinedMenuItem::separator(app)?;

    let menu = Menu::new(app)?;
    for (_, item) in &captures {
        menu.append(item)?;
    }
    menu.append_items(&[&separator, &open_item, &settings_item, &separator, &quit_item])?;
    app.manage(CaptureItems(captures));
    show_shortcuts(app, shortcuts);

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

/// Shows each capture entry's current shortcut next to it.
pub fn show_shortcuts(app: &AppHandle, shortcuts: &[ShortcutEntry]) {
    let Some(items) = app.try_state::<CaptureItems>() else { return };
    for ((_, item), entry) in items.0.iter().zip(shortcuts) {
        if let Err(error) = item.set_accelerator(entry.shortcut.as_deref()) {
            eprintln!("menu shortcut could not be shown: {error}");
        }
    }
}

/// Menu event listeners are global in Tauri: this one handles the tray and the app menu.
pub fn on_menu_event(app: &AppHandle, id: &str) {
    if let Some(kind) = id.strip_prefix(CAPTURE_PREFIX).and_then(CaptureKind::from_id) {
        capture::start(app.clone(), kind);
        return;
    }
    match id {
        "open" => editor_window::show(app, false),
        SETTINGS_ID => editor_window::show_settings(app),
        "quit" => app.exit(0),
        _ => {}
    }
}

/// Adds "Ajustes…" (⌘,) to the default app menu, after "About".
pub fn add_settings_to_app_menu(app: &AppHandle) -> tauri::Result<()> {
    let Some(menu) = app.menu() else { return Ok(()) };
    let Some(MenuItemKind::Submenu(app_menu)) = menu.items()?.into_iter().next() else { return Ok(()) };
    let settings = MenuItem::with_id(app, SETTINGS_ID, "Ajustes…", true, Some("Cmd+,"))?;
    app_menu.insert(&settings, 2)?;
    app_menu.insert(&PredefinedMenuItem::separator(app)?, 3)?;
    Ok(())
}
