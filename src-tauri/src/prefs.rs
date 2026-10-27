//! Rust side of `settings.json`. It is the same store instance the editor loads through the
//! store plugin (instances are shared by path), so neither side overwrites the other's keys.

use std::sync::Arc;

use serde::{Serialize, de::DeserializeOwned};
use tauri::{AppHandle, Wry};
use tauri_plugin_store::{Store, StoreExt};

const SETTINGS_FILE: &str = "settings.json";

fn store(app: &AppHandle) -> Result<Arc<Store<Wry>>, String> {
    // The editor writes explicitly after a debounce; so does Rust, right after each change.
    app.store_builder(SETTINGS_FILE).disable_auto_save().build().map_err(|error| error.to_string())
}

/// A missing, unreadable or malformed value is `None`, so callers fall back to defaults.
pub fn read<T: DeserializeOwned>(app: &AppHandle, key: &str) -> Option<T> {
    let value = store(app).map_err(|error| eprintln!("settings unavailable: {error}")).ok()?.get(key)?;
    serde_json::from_value(value).map_err(|error| eprintln!("setting {key} is invalid: {error}")).ok()
}

pub fn write(app: &AppHandle, key: &str, value: impl Serialize) -> Result<(), String> {
    let store = store(app)?;
    store.set(key, serde_json::to_value(value).map_err(|error| error.to_string())?);
    store.save().map_err(|error| error.to_string())
}
