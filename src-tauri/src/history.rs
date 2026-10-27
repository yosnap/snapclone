//! Capture history: one SQLite row per capture with its editing document (style and
//! annotations as JSON), so a capture can be reopened later exactly as it was left.

use std::{
    path::{Path, PathBuf},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::capture;

const DATABASE_FILE: &str = "history.sqlite3";
/// Recent captures listed in the history view.
const LIST_LIMIT: i64 = 100;
/// Documents are small JSON; anything far larger is a bug or abuse, not a real edit.
const DOCUMENT_MAX_BYTES: usize = 2 * 1024 * 1024;

pub struct History(Mutex<Connection>);

impl History {
    /// Opens the database in the app data directory. History is a convenience, so if the
    /// file cannot be opened the app keeps working with an in-memory database.
    pub fn open(app: &AppHandle) -> Self {
        let connection = app
            .path()
            .app_data_dir()
            .map_err(|error| error.to_string())
            .and_then(|dir| {
                std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
                open_database(&dir.join(DATABASE_FILE)).map_err(|error| error.to_string())
            })
            .unwrap_or_else(|error| {
                eprintln!("history database unavailable, keeping it in memory: {error}");
                let connection = Connection::open_in_memory().expect("in-memory SQLite always opens");
                migrate(&connection).expect("schema applies to an empty database");
                connection
            });
        Self(Mutex::new(connection))
    }

    fn connection(&self) -> std::sync::MutexGuard<'_, Connection> {
        // A panic while holding the lock leaves SQLite itself consistent; keep using it.
        self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn open_database(path: &Path) -> rusqlite::Result<Connection> {
    let connection = Connection::open(path)?;
    migrate(&connection)?;
    Ok(connection)
}

/// Schema steps in order; step `n` takes `user_version` from `n` to `n + 1`. Append new
/// steps, never edit shipped ones.
const MIGRATIONS: &[&str] = &["CREATE TABLE IF NOT EXISTS captures (
         id TEXT PRIMARY KEY NOT NULL,
         created_at INTEGER NOT NULL,
         path TEXT NOT NULL,
         scale REAL NOT NULL,
         document TEXT
     );
     CREATE INDEX IF NOT EXISTS captures_by_date ON captures (created_at DESC);"];

/// Applies the steps newer than `user_version`, each in its own transaction together with
/// the version bump, so running it on every start is safe.
pub fn migrate(connection: &Connection) -> rusqlite::Result<()> {
    let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    // Steps are idempotent, so an unexpected negative version just replays them.
    let applied = usize::try_from(version).unwrap_or(0);
    for (index, step) in MIGRATIONS.iter().enumerate().skip(applied) {
        connection.execute_batch(&format!("BEGIN; {step} PRAGMA user_version = {}; COMMIT;", index + 1))?;
    }
    Ok(())
}

#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryItem {
    id: String,
    /// Milliseconds since the Unix epoch.
    created_at: i64,
    scale: f64,
}

#[derive(Debug, PartialEq)]
pub struct Entry {
    pub path: PathBuf,
    pub scale: f64,
    pub document: Option<String>,
}

pub fn insert(connection: &Connection, id: &str, created_at: i64, path: &Path, scale: f64) -> rusqlite::Result<()> {
    connection.execute(
        "INSERT INTO captures (id, created_at, path, scale) VALUES (?1, ?2, ?3, ?4)",
        params![id, created_at, path.to_string_lossy(), scale],
    )?;
    Ok(())
}

/// Returns false when the id is unknown.
pub fn set_document(connection: &Connection, id: &str, document: &str) -> rusqlite::Result<bool> {
    let changed = connection.execute("UPDATE captures SET document = ?2 WHERE id = ?1", params![id, document])?;
    Ok(changed > 0)
}

/// Newest first.
pub fn list(connection: &Connection, limit: i64) -> rusqlite::Result<Vec<HistoryItem>> {
    let mut statement =
        connection.prepare("SELECT id, created_at, scale FROM captures ORDER BY created_at DESC, rowid DESC LIMIT ?1")?;
    let rows = statement.query_map([limit], |row| {
        Ok(HistoryItem { id: row.get(0)?, created_at: row.get(1)?, scale: row.get(2)? })
    })?;
    rows.collect()
}

pub fn get(connection: &Connection, id: &str) -> rusqlite::Result<Option<Entry>> {
    connection
        .query_row("SELECT path, scale, document FROM captures WHERE id = ?1", [id], |row| {
            Ok(Entry { path: PathBuf::from(row.get::<_, String>(0)?), scale: row.get(1)?, document: row.get(2)? })
        })
        .optional()
}

/// Removes the row and returns the image path it pointed to, if the id existed.
pub fn delete(connection: &Connection, id: &str) -> rusqlite::Result<Option<PathBuf>> {
    let path = get(connection, id)?.map(|entry| entry.path);
    connection.execute("DELETE FROM captures WHERE id = ?1", [id])?;
    Ok(path)
}

/// Image path of a stored capture, so `capture://` can serve captures from past sessions.
pub fn path_of(app: &AppHandle, id: &str) -> Option<PathBuf> {
    let history = app.state::<History>();
    let connection = history.connection();
    match get(&connection, id) {
        Ok(entry) => entry.map(|entry| entry.path),
        Err(error) => {
            eprintln!("history lookup failed: {error}");
            None
        }
    }
}

/// Stores a fresh capture. Failing only costs its history entry, so it is logged, not fatal.
pub fn record(app: &AppHandle, id: &str, path: &Path, scale: f64) {
    let created_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX));
    let history = app.state::<History>();
    if let Err(error) = insert(&history.connection(), id, created_at, path, scale) {
        eprintln!("capture could not be added to the history: {error}");
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenedCapture {
    id: String,
    scale: f64,
    document: Option<serde_json::Value>,
}

#[tauri::command]
pub fn list_history(history: State<'_, History>) -> Result<Vec<HistoryItem>, String> {
    list(&history.connection(), LIST_LIMIT).map_err(|error| error.to_string())
}

/// Stores the editing state of a capture. Taking a `Value` makes IPC reject anything that is
/// not JSON before it reaches the database.
#[tauri::command]
pub fn save_document(history: State<'_, History>, id: String, document: serde_json::Value) -> Result<(), String> {
    let json = document.to_string();
    if json.len() > DOCUMENT_MAX_BYTES {
        return Err("the document is too large".into());
    }
    match set_document(&history.connection(), &id, &json) {
        Ok(true) => Ok(()),
        Ok(false) => Err("unknown capture".into()),
        Err(error) => Err(error.to_string()),
    }
}

#[tauri::command]
pub fn open_history(history: State<'_, History>, id: String) -> Result<OpenedCapture, String> {
    let entry = get(&history.connection(), &id).map_err(|error| error.to_string())?.ok_or("unknown capture")?;
    if !entry.path.exists() {
        return Err("the image of this capture no longer exists".into());
    }
    // A document that no longer parses is dropped rather than blocking the image.
    let document = entry.document.and_then(|json| serde_json::from_str(&json).ok());
    Ok(OpenedCapture { id, scale: entry.scale, document })
}

#[tauri::command]
pub fn delete_history(app: AppHandle, history: State<'_, History>, id: String) -> Result<(), String> {
    let path = delete(&history.connection(), &id).map_err(|error| error.to_string())?;
    capture::forget(&app, &id);
    if let Some(path) = path {
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("the entry was removed but its image was not: {error}")),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        migrate(&connection).unwrap();
        connection
    }

    #[test]
    fn migration_is_idempotent_and_versioned() {
        let connection = database();
        insert(&connection, "a", 1, Path::new("/tmp/a.png"), 2.0).unwrap();
        migrate(&connection).unwrap();
        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0)).unwrap();
        assert_eq!(version, MIGRATIONS.len() as i64);
        assert_eq!(list(&connection, 10).unwrap().len(), 1, "re-running keeps existing rows");
    }

    #[test]
    fn lists_newest_first_with_limit() {
        let connection = database();
        for (id, at) in [("old", 10), ("new", 30), ("mid", 20)] {
            insert(&connection, id, at, Path::new("/tmp/x.png"), 1.0).unwrap();
        }
        let ids: Vec<String> = list(&connection, 2).unwrap().into_iter().map(|item| item.id).collect();
        assert_eq!(ids, ["new", "mid"]);
    }

    #[test]
    fn stores_and_reads_documents() {
        let connection = database();
        insert(&connection, "a", 1, Path::new("/tmp/a.png"), 2.0).unwrap();
        assert_eq!(get(&connection, "a").unwrap().unwrap().document, None);
        assert!(set_document(&connection, "a", r#"{"version":1}"#).unwrap());
        assert!(!set_document(&connection, "missing", "{}").unwrap());
        assert_eq!(
            get(&connection, "a").unwrap(),
            Some(Entry { path: PathBuf::from("/tmp/a.png"), scale: 2.0, document: Some(r#"{"version":1}"#.into()) })
        );
    }

    #[test]
    fn delete_returns_the_image_path_once() {
        let connection = database();
        insert(&connection, "a", 1, Path::new("/tmp/a.png"), 1.0).unwrap();
        assert_eq!(delete(&connection, "a").unwrap(), Some(PathBuf::from("/tmp/a.png")));
        assert_eq!(delete(&connection, "a").unwrap(), None);
        assert!(get(&connection, "a").unwrap().is_none());
    }

    #[test]
    fn duplicate_ids_are_rejected() {
        let connection = database();
        insert(&connection, "a", 1, Path::new("/tmp/a.png"), 1.0).unwrap();
        assert!(insert(&connection, "a", 2, Path::new("/tmp/b.png"), 1.0).is_err());
    }
}
