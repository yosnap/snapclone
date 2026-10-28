use tauri::{
    menu::{Menu, MenuItem, MenuItemKind, PredefinedMenuItem, Submenu},
    tray::TrayIconBuilder,
    AppHandle, Manager, Wry,
};

use crate::{
    capture::{self, CaptureAction},
    editor_window, import, ocr,
    shortcuts::ShortcutEntry,
};

/// Shared by the menu bar icon and the app menu, so both open the same settings.
const SETTINGS_ID: &str = "settings";
const CAPTURE_PREFIX: &str = "capture-";
const CLIPBOARD_ID: &str = "open-clipboard";
const OPEN_FILE_ID: &str = "open-file";
const REOPEN_ID: &str = "reopen-last";
const COPY_TEXT_ID: &str = "copy-last-text";
/// ⌘Q hides: the app stays in the menu bar with its shortcuts, like Xnapper.
const HIDE_ID: &str = "hide-app";
const HIDE_TITLE: &str = "Ocultar Snapclone";
const QUIT_ID: &str = "quit";
const QUIT_TITLE: &str = "Salir del todo";
/// Only a label in the menu bar icon: the editor's page handles ⇧⌘Z, where it is also redo.
const REOPEN_LABEL: &str = "Shift+Cmd+Z";

/// The capture entries, in `CaptureAction::ALL` order like the shortcut list, kept so their
/// shortcuts can be updated when they change.
struct CaptureItems(Vec<MenuItem<Wry>>);

pub fn build(app: &AppHandle, shortcuts: &[ShortcutEntry]) -> tauri::Result<()> {
    let captures = CaptureAction::ALL
        .into_iter()
        .map(|action| {
            let id = format!("{CAPTURE_PREFIX}{}", action.id());
            MenuItem::with_id(app, id, action.menu_title(), true, None::<&str>)
        })
        .collect::<tauri::Result<Vec<_>>>()?;
    let clipboard_item = MenuItem::with_id(app, CLIPBOARD_ID, "Abrir desde el portapapeles", true, None::<&str>)?;
    let file_item = MenuItem::with_id(app, OPEN_FILE_ID, "Abrir archivo…", true, None::<&str>)?;
    let reopen_item = MenuItem::with_id(app, REOPEN_ID, "Reabrir última captura", true, Some(REOPEN_LABEL))?;
    let copy_text_item = MenuItem::with_id(app, COPY_TEXT_ID, "Copiar texto de la última captura", true, None::<&str>)?;
    let text_menu = Submenu::with_items(app, "Texto detectado", true, &[&copy_text_item])?;
    let open_item = MenuItem::with_id(app, "open", "Abrir Snapclone", true, None::<&str>)?;
    let settings_item = MenuItem::with_id(app, SETTINGS_ID, "Ajustes…", true, None::<&str>)?;
    let hide_item = MenuItem::with_id(app, HIDE_ID, HIDE_TITLE, true, Some("Cmd+Q"))?;
    let quit_item = MenuItem::with_id(app, QUIT_ID, QUIT_TITLE, true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;

    let menu = Menu::new(app)?;
    for item in &captures {
        menu.append(item)?;
    }
    menu.append_items(&[&separator, &clipboard_item, &file_item, &reopen_item, &text_menu, &separator, &open_item, &settings_item, &separator, &hide_item, &quit_item])?;
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
    for (item, entry) in items.0.iter().zip(shortcuts) {
        if let Err(error) = item.set_accelerator(entry.shortcut.as_deref()) {
            eprintln!("menu shortcut could not be shown: {error}");
        }
    }
}

/// Menu event listeners are global in Tauri: this one handles the tray and the app menu.
pub fn on_menu_event(app: &AppHandle, id: &str) {
    if let Some(action) = id.strip_prefix(CAPTURE_PREFIX).and_then(CaptureAction::from_id) {
        capture::start(app.clone(), action);
        return;
    }
    match id {
        "open" => editor_window::show(app, false),
        SETTINGS_ID => editor_window::show_settings(app),
        CLIPBOARD_ID => import::open_clipboard(app),
        OPEN_FILE_ID => import::open_file_dialog(app),
        REOPEN_ID => editor_window::reopen_last(app),
        COPY_TEXT_ID => ocr::copy_latest_text(app),
        HIDE_ID => editor_window::hide_app(app),
        QUIT_ID => app.exit(0),
        _ => {}
    }
}

/// Adds "Ajustes…" (⌘,) to the default app menu, after "About", replaces its Quit with
/// "Ocultar Snapclone" (⌘Q) and "Salir del todo", and adds "Abrir archivo…" (⌘O) to its
/// File menu.
pub fn extend_app_menu(app: &AppHandle) -> tauri::Result<()> {
    let Some(menu) = app.menu() else { return Ok(()) };
    let mut submenus = menu.items()?.into_iter().filter_map(|item| match item {
        MenuItemKind::Submenu(submenu) => Some(submenu),
        _ => None,
    });
    let (Some(app_menu), Some(file_menu)) = (submenus.next(), submenus.next()) else { return Ok(()) };
    let settings = MenuItem::with_id(app, SETTINGS_ID, "Ajustes…", true, Some("Cmd+,"))?;
    app_menu.insert(&settings, 2)?;
    app_menu.insert(&PredefinedMenuItem::separator(app)?, 3)?;
    // The default app menu ends with the predefined Quit, which would end the process on ⌘Q.
    if let Some(MenuItemKind::Predefined(quit)) = app_menu.items()?.pop() {
        app_menu.remove(&quit)?;
    }
    app_menu.append(&MenuItem::with_id(app, HIDE_ID, HIDE_TITLE, true, Some("Cmd+Q"))?)?;
    app_menu.append(&MenuItem::with_id(app, QUIT_ID, QUIT_TITLE, true, None::<&str>)?)?;

    let open_file = MenuItem::with_id(app, OPEN_FILE_ID, "Abrir archivo…", true, Some("Cmd+O"))?;
    file_menu.insert(&open_file, 0)?;
    file_menu.insert(&PredefinedMenuItem::separator(app)?, 1)?;
    Ok(())
}
