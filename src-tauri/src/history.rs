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

use crate::{capture, images, ocr};

const DATABASE_FILE: &str = "history.sqlite3";
/// Captures per page of the history view; older pages load on demand, so every capture
/// can be reached and deleted.
const PAGE_SIZE: u32 = 60;
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

/// Applies the steps newer than `user_version`, so running it on every start is safe.
pub fn migrate(connection: &Connection) -> rusqlite::Result<()> {
    apply_migrations(connection, MIGRATIONS)
}

/// Each step runs in a transaction together with its version bump; a failing step rolls
/// back on drop and leaves the database at the previous version.
fn apply_migrations(connection: &Connection, steps: &[&str]) -> rusqlite::Result<()> {
    let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    // Steps are idempotent, so an unexpected negative version just replays them.
    let applied = usize::try_from(version).unwrap_or(0);
    for (index, step) in steps.iter().enumerate().skip(applied) {
        let transaction = connection.unchecked_transaction()?;
        transaction.execute_batch(step)?;
        let next = i64::try_from(index + 1).expect("migration count fits in i64");
        transaction.pragma_update(None, "user_version", next)?;
        transaction.commit()?;
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

#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryPage {
    items: Vec<HistoryItem>,
    has_more: bool,
}

/// Newest first, `limit` items after skipping `offset`.
pub fn list(connection: &Connection, offset: u32, limit: u32) -> rusqlite::Result<Vec<HistoryItem>> {
    let mut statement = connection.prepare(
        "SELECT id, created_at, scale FROM captures ORDER BY created_at DESC, rowid DESC LIMIT ?1 OFFSET ?2",
    )?;
    let rows = statement.query_map([limit, offset], |row| {
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

/// The newest capture whose image is still on disk. `exists` is the file check, injected so
/// the choice can be tested without files.
pub fn latest(connection: &Connection, exists: impl Fn(&Path) -> bool) -> rusqlite::Result<Option<String>> {
    let mut statement = connection.prepare("SELECT id, path FROM captures ORDER BY created_at DESC, rowid DESC")?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let path: String = row.get(1)?;
        if exists(Path::new(&path)) {
            return row.get(0).map(Some);
        }
    }
    Ok(None)
}

/// Id of the capture "Reabrir última captura" and "Copiar texto de la última captura" use.
pub fn latest_id(app: &AppHandle) -> Result<Option<String>, String> {
    latest(&app.state::<History>().connection(), Path::exists).map_err(|error| error.to_string())
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

/// `offset` is how many entries the view already shows; one extra row tells whether
/// there is another page.
pub fn page(connection: &Connection, offset: u32) -> rusqlite::Result<HistoryPage> {
    let mut items = list(connection, offset, PAGE_SIZE + 1)?;
    let has_more = items.len() > PAGE_SIZE as usize;
    items.truncate(PAGE_SIZE as usize);
    Ok(HistoryPage { items, has_more })
}

#[tauri::command]
pub fn list_history(history: State<'_, History>, offset: u32) -> Result<HistoryPage, String> {
    page(&history.connection(), offset).map_err(|error| error.to_string())
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
    ocr::forget(&app, &id);
    let Some(path) = path else { return Ok(()) };
    for file in [images::thumbnail_path(&path), path] {
        match std::fs::remove_file(&file) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("the entry was removed but its files were not: {error}")),
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
        assert_eq!(list(&connection, 0, 10).unwrap().len(), 1, "re-running keeps existing rows");
    }

    #[test]
    fn lists_newest_first_with_limit() {
        let connection = database();
        for (id, at) in [("old", 10), ("new", 30), ("mid", 20)] {
            insert(&connection, id, at, Path::new("/tmp/x.png"), 1.0).unwrap();
        }
        let ids: Vec<String> = list(&connection, 0, 2).unwrap().into_iter().map(|item| item.id).collect();
        assert_eq!(ids, ["new", "mid"]);
        let ids: Vec<String> = list(&connection, 2, 2).unwrap().into_iter().map(|item| item.id).collect();
        assert_eq!(ids, ["old"]);
    }

    #[test]
    fn pages_reach_every_capture() {
        let connection = database();
        let total = PAGE_SIZE + 5;
        for index in 0..total {
            insert(&connection, &format!("c{index}"), i64::from(index), Path::new("/tmp/x.png"), 1.0).unwrap();
        }
        let first = page(&connection, 0).unwrap();
        assert_eq!(first.items.len(), PAGE_SIZE as usize);
        assert!(first.has_more);
        let second = page(&connection, PAGE_SIZE).unwrap();
        assert_eq!(second.items.len(), 5);
        assert!(!second.has_more);
        assert_eq!(second.items.last().unwrap().id, "c0", "the oldest capture is reachable");
    }

    #[test]
    fn failed_migration_step_rolls_back() {
        let connection = Connection::open_in_memory().unwrap();
        let steps = ["CREATE TABLE good (id INTEGER);", "CREATE TABLE half (id INTEGER); NOT VALID SQL;"];
        assert!(apply_migrations(&connection, &steps).is_err());
        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0)).unwrap();
        assert_eq!(version, 1, "the first step stays applied");
        let half: i64 = connection
            .query_row("SELECT count(*) FROM sqlite_master WHERE name = 'half'", [], |row| row.get(0))
            .unwrap();
        assert_eq!(half, 0, "the failed step left nothing behind");
        assert!(connection.is_autocommit(), "no transaction is left open");
        apply_migrations(&connection, &["CREATE TABLE good (id INTEGER);", "CREATE TABLE half (id INTEGER);"]).unwrap();
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
    fn latest_is_the_newest_capture_whose_image_exists() {
        let connection = database();
        assert_eq!(latest(&connection, |_| true).unwrap(), None, "an empty history has none");
        for (id, at) in [("old", 10), ("gone", 30), ("mid", 20)] {
            insert(&connection, id, at, Path::new(&format!("/tmp/{id}.png")), 1.0).unwrap();
        }
        assert_eq!(latest(&connection, |_| true).unwrap().as_deref(), Some("gone"));
        let without_gone = |path: &Path| path != Path::new("/tmp/gone.png");
        assert_eq!(latest(&connection, without_gone).unwrap().as_deref(), Some("mid"));
        assert_eq!(latest(&connection, |_| false).unwrap(), None);
    }

    #[test]
    fn latest_breaks_timestamp_ties_by_insertion_order() {
        let connection = database();
        insert(&connection, "first", 5, Path::new("/tmp/a.png"), 1.0).unwrap();
        insert(&connection, "second", 5, Path::new("/tmp/b.png"), 1.0).unwrap();
        assert_eq!(latest(&connection, |_| true).unwrap().as_deref(), Some("second"));
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
