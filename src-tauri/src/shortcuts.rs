//! One global shortcut per capture action. They are saved in `settings.json` and re-registered
//! at once when changed; parsing and validation are pure so they are tested without the OS.
//! Every command here is synchronous, so it runs on the main thread like the shortcut
//! handler and the registry calls, and nothing can interleave with a change in progress.

use std::{
    collections::HashMap,
    str::FromStr,
    sync::Mutex,
};

use serde::Serialize;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut};

use crate::{capture::{self, CaptureAction}, prefs, tray};

const SHORTCUTS_KEY: &str = "shortcuts";
/// The system screenshot shortcuts, which macOS keeps for itself.
const RESERVED: [(Modifiers, Code); 3] = [
    (Modifiers::SUPER.union(Modifiers::SHIFT), Code::Digit3),
    (Modifiers::SUPER.union(Modifiers::SHIFT), Code::Digit4),
    (Modifiers::SUPER.union(Modifiers::SHIFT), Code::Digit5),
];
/// The modifiers a shortcut can be made of, in the order macOS shows them.
const MODIFIERS: [(Modifiers, &str, &str); 4] = [
    (Modifiers::CONTROL, "Ctrl", "⌃"),
    (Modifiers::ALT, "Alt", "⌥"),
    (Modifiers::SHIFT, "Shift", "⇧"),
    (Modifiers::SUPER, "Cmd", "⌘"),
];

/// Like the original Xnapper: a zone opens in the editor with ⌃⌥⌘4 and is copied with
/// ⌃⌥⌘5; the rest are ⇧⌘ plus a digit.
fn default_shortcut(action: CaptureAction) -> Shortcut {
    let zone = |key| Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SUPER), key);
    let key = match action {
        CaptureAction::Region => return zone(Code::Digit4),
        CaptureAction::RegionCopy => return zone(Code::Digit5),
        CaptureAction::Window => Code::Digit7,
        CaptureAction::Screen => Code::Digit8,
        CaptureAction::Scroll => Code::Digit9,
    };
    Shortcut::new(Some(Modifiers::SUPER | Modifiers::SHIFT), key)
}

type Assigned = HashMap<CaptureAction, Option<Shortcut>>;

#[derive(Default)]
pub struct Shortcuts(Mutex<Registry>);

#[derive(Default)]
struct Registry {
    assigned: Assigned,
    /// Shortcuts that are saved but could not be registered, with the reason.
    failed: HashMap<CaptureAction, String>,
    /// While the settings record a new combination nothing is registered, so the keys reach
    /// the recorder instead of starting a capture.
    paused: bool,
}

/// What the settings and the menu show for one capture action.
#[derive(Clone, Serialize)]
pub struct ShortcutEntry {
    action: CaptureAction,
    /// The capture action as the user reads it, from `CaptureAction::label`.
    name: &'static str,
    /// Stored form, e.g. `Shift+Cmd+Digit2`, which the menu also takes as its accelerator.
    pub shortcut: Option<String>,
    /// As macOS shows it, e.g. `⇧⌘2`.
    label: Option<String>,
    error: Option<String>,
}

/// Stored form: modifiers in a fixed order, then the key code.
pub fn canonical(shortcut: &Shortcut) -> String {
    let mut parts: Vec<String> = MODIFIERS
        .iter()
        .filter(|(modifier, ..)| shortcut.mods.contains(*modifier))
        .map(|(_, name, _)| (*name).to_string())
        .collect();
    parts.push(shortcut.key.to_string());
    parts.join("+")
}

/// How macOS writes a shortcut, e.g. `⌃⇧⌘A`.
pub fn label(shortcut: &Shortcut) -> String {
    let mut text: String = MODIFIERS
        .iter()
        .filter(|(modifier, ..)| shortcut.mods.contains(*modifier))
        .map(|(.., symbol)| *symbol)
        .collect();
    text.push_str(&key_label(shortcut.key));
    text
}

fn key_label(key: Code) -> String {
    let name = key.to_string();
    if let Some(rest) = name.strip_prefix("Digit").or_else(|| name.strip_prefix("Key")) {
        return rest.to_string();
    }
    let symbol = match key {
        Code::Space => "Espacio",
        Code::Enter => "↩",
        Code::Tab => "⇥",
        Code::Backspace => "⌫",
        Code::Delete => "⌦",
        Code::Escape => "⎋",
        Code::ArrowUp => "↑",
        Code::ArrowDown => "↓",
        Code::ArrowLeft => "←",
        Code::ArrowRight => "→",
        Code::Home => "↖",
        Code::End => "↘",
        Code::PageUp => "⇞",
        Code::PageDown => "⇟",
        Code::Minus => "-",
        Code::Equal => "=",
        Code::Comma => ",",
        Code::Period => ".",
        Code::Slash => "/",
        Code::Backslash => "\\",
        Code::Semicolon => ";",
        Code::Quote => "'",
        Code::Backquote => "`",
        Code::BracketLeft => "[",
        Code::BracketRight => "]",
        _ => return name.replace("Numpad", "Num "),
    };
    symbol.to_string()
}

/// Parses a shortcut from the settings file or the recorder. It must have ⌘, ⌥ or ⌃: a bare
/// key, or one with only ⇧ (which just types a capital or a symbol), registered globally
/// would stop working for typing in every other app.
pub fn parse(text: &str) -> Result<Shortcut, String> {
    let shortcut = Shortcut::from_str(text).map_err(|_| "Esa tecla no se puede usar en un atajo".to_string())?;
    if !shortcut.mods.intersects(Modifiers::SUPER | Modifiers::ALT | Modifiers::CONTROL) {
        return Err("El atajo debe llevar ⌘, ⌥ o ⌃ (⇧ solo no basta)".into());
    }
    Ok(shortcut)
}

/// Rejects the system screenshot shortcuts and any shortcut another capture action already has.
pub fn validate(action: CaptureAction, candidate: &Shortcut, assigned: &Assigned) -> Result<(), String> {
    if RESERVED.iter().any(|&(mods, key)| candidate.mods == mods && candidate.key == key) {
        return Err(format!("{} está reservado por macOS para sus capturas de pantalla", label(candidate)));
    }
    let taken = CaptureAction::ALL
        .into_iter()
        .find(|other| *other != action && assigned.get(other).copied().flatten().is_some_and(|used| used.id() == candidate.id()));
    match taken {
        Some(other) => Err(format!("{} ya se usa para «{}»", label(candidate), other.label())),
        None => Ok(()),
    }
}

/// Reads the saved shortcuts. An action missing from the file gets its default; `null` means
/// the user cleared it. An invalid entry falls back to the default. A saved choice always
/// wins: the saved ones are taken first, and a default that clashes with one of them (say a
/// zone shortcut saved as ⌃⌥⌘5 before the copy action existed) is dropped, like a saved one
/// that clashes with an earlier action.
fn load(saved: Option<HashMap<String, Option<String>>>) -> Assigned {
    let saved = saved.unwrap_or_default();
    let candidates: Vec<(CaptureAction, bool, Option<Shortcut>)> = CaptureAction::ALL
        .into_iter()
        .map(|action| match saved.get(action.id()) {
            None => (action, false, Some(default_shortcut(action))),
            Some(None) => (action, true, None),
            Some(Some(text)) => {
                let shortcut = parse(text).unwrap_or_else(|error| {
                    eprintln!("saved shortcut {text:?} for {} is invalid: {error}", action.id());
                    default_shortcut(action)
                });
                (action, true, Some(shortcut))
            }
        })
        .collect();
    let mut assigned = Assigned::new();
    for pass_saved in [true, false] {
        for (action, is_saved, candidate) in &candidates {
            if *is_saved != pass_saved {
                continue;
            }
            let usable = candidate.filter(|shortcut| validate(*action, shortcut, &assigned).is_ok());
            assigned.insert(*action, usable);
        }
    }
    assigned
}

/// Registers every assigned shortcut, returning those the system refused.
fn register_all(app: &AppHandle, assigned: &Assigned) -> HashMap<CaptureAction, String> {
    let manager = app.global_shortcut();
    let mut failed = HashMap::new();
    for action in CaptureAction::ALL {
        let Some(shortcut) = assigned.get(&action).copied().flatten() else { continue };
        if let Err(error) = manager.register(shortcut) {
            eprintln!("shortcut {} could not be registered: {error}", canonical(&shortcut));
            failed.insert(action, format!("No se pudo registrar {}: puede que otra app ya lo use", label(&shortcut)));
        }
    }
    failed
}

fn unregister_all(app: &AppHandle) {
    if let Err(error) = app.global_shortcut().unregister_all() {
        eprintln!("shortcuts could not be unregistered: {error}");
    }
}

fn entries(registry: &Registry) -> Vec<ShortcutEntry> {
    CaptureAction::ALL
        .into_iter()
        .map(|action| {
            let shortcut = registry.assigned.get(&action).copied().flatten();
            ShortcutEntry {
                action,
                name: action.label(),
                shortcut: shortcut.as_ref().map(canonical),
                label: shortcut.as_ref().map(label),
                error: registry.failed.get(&action).cloned(),
            }
        })
        .collect()
}

/// Loads and registers the saved shortcuts at launch; returns what the menu should show.
pub fn init(app: &AppHandle) -> Vec<ShortcutEntry> {
    let assigned = load(prefs::read(app, SHORTCUTS_KEY));
    let failed = register_all(app, &assigned);
    let state = app.state::<Shortcuts>();
    let mut registry = state.0.lock().unwrap();
    *registry = Registry { assigned, failed, paused: false };
    entries(&registry)
}

/// Starts the capture action a pressed shortcut belongs to.
pub fn on_shortcut(app: &AppHandle, shortcut: &Shortcut) {
    let action = {
        let state = app.state::<Shortcuts>();
        let registry = state.0.lock().unwrap();
        if registry.paused {
            return;
        }
        CaptureAction::ALL
            .into_iter()
            .find(|action| registry.assigned.get(action).copied().flatten().is_some_and(|used| used.id() == shortcut.id()))
    };
    if let Some(action) = action {
        capture::start(app.clone(), action);
    }
}

#[tauri::command]
pub fn get_shortcuts(state: State<'_, Shortcuts>) -> Vec<ShortcutEntry> {
    entries(&state.0.lock().unwrap())
}

/// Assigns (or clears, with `None`) the shortcut of a capture action. The old one is released
/// and the new one registered; if the system refuses it, the previous shortcuts are restored
/// and the error is returned. Registering also ends a pause.
#[tauri::command]
pub fn set_shortcut(app: AppHandle, action: CaptureAction, shortcut: Option<String>) -> Result<Vec<ShortcutEntry>, String> {
    let candidate = shortcut.as_deref().map(parse).transpose()?;
    let state = app.state::<Shortcuts>();
    let previous = state.0.lock().unwrap().assigned.clone();
    if let Some(candidate) = &candidate {
        validate(action, candidate, &previous)?;
    }
    let mut next = previous.clone();
    next.insert(action, candidate);

    unregister_all(&app);
    let mut failed = register_all(&app, &next);
    if let Some(error) = failed.remove(&action) {
        unregister_all(&app);
        let restored = register_all(&app, &previous);
        let mut registry = state.0.lock().unwrap();
        registry.failed = restored;
        registry.paused = false;
        return Err(error);
    }

    let saved: HashMap<&str, Option<String>> =
        next.iter().map(|(action, shortcut)| (action.id(), shortcut.as_ref().map(canonical))).collect();
    let list = {
        let mut registry = state.0.lock().unwrap();
        *registry = Registry { assigned: next, failed, paused: false };
        entries(&registry)
    };
    tray::show_shortcuts(&app, &list);
    // Already in effect; only persisting failed, so the change is kept for this session.
    prefs::write(&app, SHORTCUTS_KEY, saved).map_err(|error| format!("El atajo funciona, pero no se pudo guardar: {error}"))?;
    Ok(list)
}

/// Pauses every global shortcut while the settings record a combination, and resumes them.
#[tauri::command]
pub fn pause_shortcuts(app: AppHandle, paused: bool) {
    if paused { pause(&app) } else { resume(&app) }
}

fn pause(app: &AppHandle) {
    let state = app.state::<Shortcuts>();
    if std::mem::replace(&mut state.0.lock().unwrap().paused, true) {
        return;
    }
    unregister_all(app);
}

/// Also called when the editor loses focus, so a recorder left open never leaves the app
/// without its shortcuts.
pub fn resume(app: &AppHandle) {
    let state = app.state::<Shortcuts>();
    let assigned = {
        let mut registry = state.0.lock().unwrap();
        if !std::mem::replace(&mut registry.paused, false) {
            return;
        }
        registry.assigned.clone()
    };
    let failed = register_all(app, &assigned);
    state.0.lock().unwrap().failed = failed;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shortcut(text: &str) -> Shortcut {
        parse(text).unwrap()
    }

    fn defaults() -> Assigned {
        load(None)
    }

    fn text(assigned: &Assigned, action: CaptureAction) -> Option<String> {
        assigned[&action].as_ref().map(canonical)
    }

    #[test]
    fn defaults_follow_xnapper_then_cmd_shift_seven_eight_nine() {
        let assigned = defaults();
        assert_eq!(text(&assigned, CaptureAction::Region).as_deref(), Some("Ctrl+Alt+Cmd+Digit4"));
        assert_eq!(text(&assigned, CaptureAction::RegionCopy).as_deref(), Some("Ctrl+Alt+Cmd+Digit5"));
        assert_eq!(text(&assigned, CaptureAction::Window).as_deref(), Some("Shift+Cmd+Digit7"));
        assert_eq!(text(&assigned, CaptureAction::Screen).as_deref(), Some("Shift+Cmd+Digit8"));
        assert_eq!(text(&assigned, CaptureAction::Scroll).as_deref(), Some("Shift+Cmd+Digit9"));
    }

    #[test]
    fn action_ids_match_their_serialized_form() {
        for action in CaptureAction::ALL {
            assert_eq!(serde_json::to_value(action).unwrap(), action.id());
            assert_eq!(CaptureAction::from_id(action.id()), Some(action));
        }
    }

    #[test]
    fn canonical_form_round_trips_and_labels_like_macos() {
        let parsed = shortcut("Cmd+Ctrl+Shift+KeyA");
        assert_eq!(canonical(&parsed), "Ctrl+Shift+Cmd+KeyA");
        assert_eq!(parse(&canonical(&parsed)).unwrap(), parsed);
        assert_eq!(label(&parsed), "⌃⇧⌘A");
        assert_eq!(label(&shortcut("Alt+Space")), "⌥Espacio");
    }

    #[test]
    fn shortcuts_need_a_modifier_and_a_known_key() {
        assert!(parse("KeyA").is_err());
        assert!(parse("Cmd+Shift+NotAKey").is_err());
        assert!(parse("Shift+KeyA").is_err());
        assert!(parse("Alt+KeyA").is_ok());
        assert!(parse("Ctrl+Shift+KeyA").is_ok());
    }

    #[test]
    fn system_screenshot_shortcuts_are_reserved() {
        for key in ["Digit3", "Digit4", "Digit5"] {
            let error = validate(CaptureAction::Region, &shortcut(&format!("Cmd+Shift+{key}")), &defaults()).unwrap_err();
            assert!(error.contains("reservado"), "{error}");
        }
        assert!(validate(CaptureAction::Region, &shortcut("Cmd+Alt+Digit3"), &defaults()).is_ok());
    }

    #[test]
    fn a_shortcut_used_by_another_action_conflicts() {
        let error = validate(CaptureAction::Region, &shortcut("Shift+Cmd+Digit7"), &defaults()).unwrap_err();
        assert_eq!(error, "⇧⌘7 ya se usa para «Capturar ventana»");
        let error = validate(CaptureAction::Region, &shortcut("Ctrl+Alt+Cmd+Digit5"), &defaults()).unwrap_err();
        assert_eq!(error, "⌃⌥⌘5 ya se usa para «Capturar y copiar al portapapeles»");
        // Re-assigning an action its own shortcut is fine.
        assert!(validate(CaptureAction::Window, &shortcut("Shift+Cmd+Digit7"), &defaults()).is_ok());
    }

    #[test]
    fn saved_shortcuts_override_defaults_and_bad_ones_fall_back() {
        let saved = HashMap::from([
            ("region".to_string(), Some("Alt+Cmd+KeyR".to_string())),
            ("window".to_string(), None),
            ("screen".to_string(), Some("garbage".to_string())),
            // Clashes with the region shortcut above, so it is dropped.
            ("scroll".to_string(), Some("Alt+Cmd+KeyR".to_string())),
        ]);
        let assigned = load(Some(saved));
        assert_eq!(text(&assigned, CaptureAction::Region).as_deref(), Some("Alt+Cmd+KeyR"));
        assert_eq!(assigned[&CaptureAction::Window], None);
        assert_eq!(assigned[&CaptureAction::Screen], Some(default_shortcut(CaptureAction::Screen)));
        assert_eq!(assigned[&CaptureAction::Scroll], None);
        assert_eq!(assigned[&CaptureAction::RegionCopy], Some(default_shortcut(CaptureAction::RegionCopy)));
    }

    #[test]
    fn an_old_saved_zone_shortcut_is_kept() {
        let saved = HashMap::from([("region".to_string(), Some("Shift+Cmd+Digit2".to_string()))]);
        let assigned = load(Some(saved));
        assert_eq!(text(&assigned, CaptureAction::Region).as_deref(), Some("Shift+Cmd+Digit2"));
        assert_eq!(assigned[&CaptureAction::Window], Some(default_shortcut(CaptureAction::Window)));
    }

    #[test]
    fn a_saved_choice_beats_the_copy_default() {
        // Saved when ⌃⌥⌘5 was the zone default: the zone keeps it and copying gets none.
        let saved = HashMap::from([("region".to_string(), Some("Ctrl+Alt+Cmd+Digit5".to_string()))]);
        let assigned = load(Some(saved));
        assert_eq!(text(&assigned, CaptureAction::Region).as_deref(), Some("Ctrl+Alt+Cmd+Digit5"));
        assert_eq!(assigned[&CaptureAction::RegionCopy], None);

        // A later action's saved choice also wins over an earlier action's default.
        let saved = HashMap::from([("scroll".to_string(), Some("Ctrl+Alt+Cmd+Digit4".to_string()))]);
        let assigned = load(Some(saved));
        assert_eq!(text(&assigned, CaptureAction::Scroll).as_deref(), Some("Ctrl+Alt+Cmd+Digit4"));
        assert_eq!(assigned[&CaptureAction::Region], None);
    }
}
