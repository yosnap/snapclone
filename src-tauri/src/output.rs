//! Where exported images go: the remembered save folder ("Guardar ⌘S"), the default file
//! name, the temporary file shared or dragged out of the editor, and showing them in the
//! Finder. Every path is decided here; the page only sends the encoded image and the local
//! date and time it was exported at, which is validated before it becomes part of a name.

use std::{
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex},
};

use regex::Regex;
use tauri::{
    AppHandle, Manager, State,
    ipc::{InvokeBody, Request},
};
use tauri_plugin_dialog::DialogExt;

use crate::prefs;

const SAVE_FOLDER_KEY: &str = "saveFolder";
const FILE_NAME_KEY: &str = "defaultFileName";
pub const DEFAULT_PATTERN: &str = "Snapclone {datetime}";
const DATETIME_HEADER: &str = "x-datetime";
const FILE_NAME_MAX_CHARS: usize = 120;
const EXPORT_DIR: &str = "export";
/// Never overwrite: "name (2).png", "name (3).png"… up to this many.
const MAX_SUFFIX: u32 = 999;

/// As the editor formats it: "2026-10-28 a las 00.24.05".
static DATETIME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\d{4}-\d{2}-\d{2} a las \d{2}\.\d{2}\.\d{2}$").expect("valid regex"));

/// The last file written by Guardar or Guardar como, for "Abrir en el Finder".
#[derive(Default)]
pub struct LastSaved(pub Mutex<Option<PathBuf>>);

/// The image last prepared for sharing or dragging out.
#[derive(Default)]
pub struct ExportFile(pub Mutex<Option<PathBuf>>);

pub fn raw_body<'a>(request: &'a Request<'_>) -> Result<&'a [u8], String> {
    match request.body() {
        InvokeBody::Raw(bytes) if !bytes.is_empty() => Ok(bytes),
        _ => Err("expected raw image bytes".into()),
    }
}

/// The export time sent by the page, or an error if it is not exactly a date and a time.
pub fn checked_datetime(value: &str) -> Result<&str, String> {
    if DATETIME.is_match(value) { Ok(value) } else { Err(format!("invalid export time: {value:?}")) }
}

fn request_datetime<'a>(request: &'a Request<'_>) -> Result<&'a str, String> {
    let value = request.headers().get(DATETIME_HEADER).ok_or("missing export time")?;
    checked_datetime(value.to_str().map_err(|error| error.to_string())?)
}

pub fn file_name_pattern(app: &AppHandle) -> String {
    prefs::read::<String>(app, FILE_NAME_KEY).filter(|value| !value.trim().is_empty()).unwrap_or_else(|| DEFAULT_PATTERN.into())
}

/// Builds a file name (without extension) from the user's pattern. Characters that would
/// change the folder or hide the file are replaced, so the name always stays a single,
/// visible entry in the chosen folder.
pub fn file_stem(pattern: &str, datetime: &str, uuid: &str) -> String {
    let expanded = pattern.replace("{datetime}", datetime).replace("{uuid}", uuid);
    let cleaned: String = expanded
        .chars()
        .map(|character| if matches!(character, '/' | ':' | '\\') || character.is_control() { '-' } else { character })
        .collect();
    let trimmed = cleaned.trim().trim_start_matches('.').trim();
    let stem: String = trimmed.chars().take(FILE_NAME_MAX_CHARS).collect();
    if stem.is_empty() { DEFAULT_PATTERN.replace("{datetime}", datetime) } else { stem }
}

fn default_file_name(app: &AppHandle, datetime: &str) -> String {
    format!("{}.png", file_stem(&file_name_pattern(app), datetime, &uuid::Uuid::new_v4().to_string()))
}

/// `dir/name`, or the first "name (n).ext" that does not exist yet.
pub fn unique_path(dir: &Path, name: &str, exists: impl Fn(&Path) -> bool) -> Result<PathBuf, String> {
    let candidate = dir.join(name);
    if !exists(&candidate) {
        return Ok(candidate);
    }
    let (stem, extension) = match name.rsplit_once('.') {
        Some((stem, extension)) => (stem, format!(".{extension}")),
        None => (name, String::new()),
    };
    (2..=MAX_SUFFIX)
        .map(|index| dir.join(format!("{stem} ({index}){extension}")))
        .find(|path| !exists(path))
        .ok_or_else(|| format!("there are already too many files named {name}"))
}

/// The remembered folder, if it is still there.
pub fn save_folder(app: &AppHandle) -> Option<PathBuf> {
    prefs::read::<String>(app, SAVE_FOLDER_KEY).map(PathBuf::from).filter(|path| path.is_dir())
}

/// Asks for a folder with the system dialog and remembers it. `None` when cancelled.
fn pick_save_folder(app: &AppHandle) -> Result<Option<PathBuf>, String> {
    let mut dialog = app.dialog().file().set_title("Carpeta donde guardar las capturas");
    if let Some(current) = save_folder(app) {
        dialog = dialog.set_directory(current);
    }
    let Some(chosen) = dialog.blocking_pick_folder() else { return Ok(None) };
    let path = chosen.into_path().map_err(|error| error.to_string())?;
    prefs::write(app, SAVE_FOLDER_KEY, path.to_string_lossy())?;
    Ok(Some(path))
}

#[tauri::command]
pub async fn choose_save_folder(app: AppHandle) -> Result<Option<String>, String> {
    Ok(pick_save_folder(&app)?.map(|path| path.to_string_lossy().into_owned()))
}

#[tauri::command]
pub fn set_file_name_pattern(app: AppHandle, pattern: String) -> Result<String, String> {
    let pattern = pattern.trim();
    let stored = if pattern.is_empty() { DEFAULT_PATTERN } else { pattern };
    if stored.chars().count() > FILE_NAME_MAX_CHARS {
        return Err(format!("el nombre no puede pasar de {FILE_NAME_MAX_CHARS} caracteres"));
    }
    prefs::write(&app, FILE_NAME_KEY, stored)?;
    Ok(stored.to_string())
}

async fn write_file(path: PathBuf, bytes: Vec<u8>) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || std::fs::write(path, bytes))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}

/// "Guardar ⌘S": writes a PNG to the save folder under the default name. Without a folder,
/// asks for one once and remembers it. Returns the path written, or `None` if cancelled.
#[tauri::command]
pub async fn quick_save(app: AppHandle, last: State<'_, LastSaved>, request: Request<'_>) -> Result<Option<String>, String> {
    let datetime = request_datetime(&request)?.to_string();
    let bytes = raw_body(&request)?.to_vec();
    let folder = match save_folder(&app) {
        Some(folder) => folder,
        None => match pick_save_folder(&app)? {
            Some(folder) => folder,
            None => return Ok(None),
        },
    };
    let path = unique_path(&folder, &default_file_name(&app, &datetime), |path| path.exists())?;
    write_file(path.clone(), bytes).await?;
    let display = path.to_string_lossy().into_owned();
    *last.0.lock().unwrap() = Some(path);
    Ok(Some(display))
}

/// The default name and folder offered by "Guardar como…".
pub fn save_dialog_defaults(app: &AppHandle, datetime: &str) -> Result<(String, Option<PathBuf>), String> {
    Ok((default_file_name(app, checked_datetime(datetime)?), save_folder(app)))
}

pub fn remember_saved(app: &AppHandle, path: PathBuf) {
    *app.state::<LastSaved>().0.lock().unwrap() = Some(path);
}

/// "Abrir en el Finder": the last saved file, selected, or else the save folder.
#[tauri::command]
pub fn reveal_saved(app: AppHandle, last: State<'_, LastSaved>) -> Result<(), String> {
    let saved = last.0.lock().unwrap().clone().filter(|path| path.exists());
    match (saved, save_folder(&app)) {
        (Some(file), _) => open_in_finder(&file, true),
        (None, Some(folder)) => open_in_finder(&folder, false),
        (None, None) => Err("todavía no has guardado ninguna imagen".into()),
    }
}

#[tauri::command]
pub fn open_save_folder(app: AppHandle) -> Result<(), String> {
    open_in_finder(&save_folder(&app).ok_or("no hay carpeta de guardado")?, false)
}

/// Opens a folder, or selects a file in its folder, in the Finder.
pub fn open_in_finder(path: &Path, select: bool) -> Result<(), String> {
    let mut command = std::process::Command::new("/usr/bin/open");
    if select {
        command.arg("-R");
    }
    let status = command.arg(path).status().map_err(|error| error.to_string())?;
    if status.success() { Ok(()) } else { Err(format!("the Finder could not open {}", path.display())) }
}

/// "Enviar comentarios": an empty message in the default mail app.
#[tauri::command]
pub fn send_feedback() -> Result<(), String> {
    let status = std::process::Command::new("/usr/bin/open")
        .arg("mailto:?subject=Snapclone")
        .status()
        .map_err(|error| error.to_string())?;
    if status.success() { Ok(()) } else { Err("no hay ninguna app de correo configurada".into()) }
}

/// Writes the image to share or drag out to a private folder under the default name, so it
/// arrives with a meaningful name. Only the latest one is kept.
#[tauri::command]
pub async fn prepare_export(app: AppHandle, export: State<'_, ExportFile>, request: Request<'_>) -> Result<(), String> {
    let datetime = request_datetime(&request)?.to_string();
    let bytes = raw_body(&request)?.to_vec();
    let dir = app.path().app_cache_dir().map_err(|error| error.to_string())?.join(EXPORT_DIR);
    let path = dir.join(default_file_name(&app, &datetime));
    let target = path.clone();
    tauri::async_runtime::spawn_blocking(move || -> std::io::Result<()> {
        // Older exports are left from previous shares and drags; they are no longer needed.
        if dir.exists() {
            std::fs::remove_dir_all(&dir)?;
        }
        std::fs::create_dir_all(&dir)?;
        std::fs::write(&target, bytes)
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| error.to_string())?;
    *export.0.lock().unwrap() = Some(path);
    Ok(())
}

pub fn export_file(app: &AppHandle) -> Result<PathBuf, String> {
    app.state::<ExportFile>().0.lock().unwrap().clone().filter(|path| path.exists()).ok_or_else(|| "no image was prepared".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    const WHEN: &str = "2026-10-28 a las 00.24.05";

    #[test]
    fn datetime_must_be_exact() {
        assert!(checked_datetime(WHEN).is_ok());
        assert!(checked_datetime("2026-10-28 a las 00.24.05/../x").is_err());
        assert!(checked_datetime("../../etc").is_err());
        assert!(checked_datetime("").is_err());
    }

    #[test]
    fn stem_expands_variables() {
        assert_eq!(file_stem(DEFAULT_PATTERN, WHEN, "u"), format!("Snapclone {WHEN}"));
        assert_eq!(file_stem("captura-{uuid}", WHEN, "abc"), "captura-abc");
    }

    #[test]
    fn stem_never_leaves_the_folder_or_hides_the_file() {
        assert_eq!(file_stem("../../secret", WHEN, "u"), "-..-secret");
        assert_eq!(file_stem(".hidden", WHEN, "u"), "hidden");
        assert_eq!(file_stem("a:b\\c\n", WHEN, "u"), "a-b-c-");
        assert_eq!(file_stem("   ", WHEN, "u"), format!("Snapclone {WHEN}"));
        assert_eq!(file_stem(&"x".repeat(500), WHEN, "u").chars().count(), FILE_NAME_MAX_CHARS);
    }

    #[test]
    fn unique_path_adds_a_counter() {
        let dir = Path::new("/folder");
        let taken = [dir.join("a.png"), dir.join("a (2).png")];
        let exists = |path: &Path| taken.iter().any(|item| item == path);
        assert_eq!(unique_path(dir, "b.png", exists).unwrap(), dir.join("b.png"));
        assert_eq!(unique_path(dir, "a.png", exists).unwrap(), dir.join("a (3).png"));
        assert!(unique_path(dir, "a.png", |_| true).is_err());
    }
}
