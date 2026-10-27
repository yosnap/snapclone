use tauri::{
    menu::{Menu, MenuItem, MenuItemKind, PredefinedMenuItem, Submenu},
    tray::TrayIconBuilder,
    AppHandle, Manager, Wry,
};

use crate::{
    capture::{self, CaptureKind},
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
/// The app menu's copy, whose ⇧⌘Z comes and goes; see `editor_window::sync_reopen_shortcut`.
const REOPEN_SHORTCUT_ID: &str = "reopen-last-shortcut";
const REOPEN_ACCELERATOR: &str = "Shift+Cmd+Z";

/// The app menu entry whose shortcut is switched on and off.
struct ReopenItem(MenuItem<Wry>);

/// The capture entries, kept so their shortcuts can be updated when they change.
struct CaptureItems(Vec<(CaptureKind, MenuItem<Wry>)>);

/// "Capturar zona", "Capturar con scroll"…, from the one name each kind has.
fn capture_title(kind: CaptureKind) -> String {
    format!("Capturar {}", kind.label().to_lowercase())
}

pub fn build(app: &AppHandle, shortcuts: &[ShortcutEntry]) -> tauri::Result<()> {
    let captures = CaptureKind::ALL
        .into_iter()
        .map(|kind| {
            let id = format!("{CAPTURE_PREFIX}{}", kind.id());
            MenuItem::with_id(app, id, capture_title(kind), true, None::<&str>).map(|item| (kind, item))
        })
        .collect::<tauri::Result<Vec<_>>>()?;
    let clipboard_item = MenuItem::with_id(app, CLIPBOARD_ID, "Abrir desde el portapapeles", true, None::<&str>)?;
    let file_item = MenuItem::with_id(app, OPEN_FILE_ID, "Abrir archivo…", true, None::<&str>)?;
    // The menu bar icon only shows the shortcut; it acts through the app menu.
    let reopen_item = MenuItem::with_id(app, REOPEN_ID, "Reabrir última captura", true, Some(REOPEN_ACCELERATOR))?;
    let copy_text_item = MenuItem::with_id(app, COPY_TEXT_ID, "Copiar texto de la última captura", true, None::<&str>)?;
    let text_menu = Submenu::with_items(app, "Texto detectado", true, &[&copy_text_item])?;
    let open_item = MenuItem::with_id(app, "open", "Abrir Snapclone", true, None::<&str>)?;
    let settings_item = MenuItem::with_id(app, SETTINGS_ID, "Ajustes…", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "Salir", true, Some("Cmd+Q"))?;
    let separator = PredefinedMenuItem::separator(app)?;

    let menu = Menu::new(app)?;
    for (_, item) in &captures {
        menu.append(item)?;
    }
    menu.append_items(&[&separator, &clipboard_item, &file_item, &reopen_item, &text_menu, &separator, &open_item, &settings_item, &separator, &quit_item])?;
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
        CLIPBOARD_ID => import::open_clipboard(app),
        OPEN_FILE_ID => import::open_file_dialog(app),
        REOPEN_ID => editor_window::reopen_last(app),
        COPY_TEXT_ID => ocr::copy_latest_text(app),
        REOPEN_SHORTCUT_ID => editor_window::reopen_shortcut(app),
        "quit" => app.exit(0),
        _ => {}
    }
}

/// Adds "Ajustes…" (⌘,) to the default app menu, after "About", and "Abrir archivo…" (⌘O)
/// and "Reabrir última captura" (⇧⌘Z) to its File menu, which comes before Edit so its ⇧⌘Z
/// wins over Redo while it is on.
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

    let open_file = MenuItem::with_id(app, OPEN_FILE_ID, "Abrir archivo…", true, Some("Cmd+O"))?;
    // Starts on: the editor has no capture until the page reports one.
    let reopen = MenuItem::with_id(app, REOPEN_SHORTCUT_ID, "Reabrir última captura", true, Some(REOPEN_ACCELERATOR))?;
    file_menu.insert(&open_file, 0)?;
    file_menu.insert(&reopen, 1)?;
    file_menu.insert(&PredefinedMenuItem::separator(app)?, 2)?;
    app.manage(ReopenItem(reopen));
    Ok(())
}

/// Gives the app menu's "Reabrir última captura" its ⇧⌘Z, or takes it away so it is redo.
pub fn set_reopen_shortcut(app: &AppHandle, enabled: bool) {
    let Some(item) = app.try_state::<ReopenItem>() else { return };
    if let Err(error) = item.0.set_accelerator(enabled.then_some(REOPEN_ACCELERATOR)) {
        eprintln!("reopen shortcut could not be updated: {error}");
    }
}
