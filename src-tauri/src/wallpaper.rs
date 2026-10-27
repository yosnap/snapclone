//! The desktop picture of the main display, which the editor blurs into a background like
//! Xnapper does. Its path comes from AppKit; the webview only ever asks for
//! `capture://localhost/wallpaper` and can never name a file.

use std::{
    hash::{DefaultHasher, Hash, Hasher},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, SystemTime},
};

use tauri::{AppHandle, Manager};

/// AppKit is asked on the main thread; this bounds the wait if that thread is busy.
const MAIN_THREAD_TIMEOUT: Duration = Duration::from_secs(2);

/// Formats WebKit decodes, by file extension, with the type they are served as.
const CONTENT_TYPES: [(&str, &str); 7] = [
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("heic", "image/heic"),
    ("heif", "image/heif"),
    ("tif", "image/tiff"),
    ("tiff", "image/tiff"),
];

/// Type to serve a wallpaper file as; `None` for folders (rotating or dynamic wallpapers)
/// and formats the editor cannot draw.
pub fn content_type(path: &Path) -> Option<&'static str> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    CONTENT_TYPES.iter().find(|(known, _)| *known == extension).map(|(_, kind)| *kind)
}

/// What `capture://localhost/wallpaper…` serves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variant {
    /// The picture as it is on disk.
    Full,
    /// A small PNG for the background swatch.
    Thumbnail,
    /// Only the version, so the editor can tell whether its copy is still current.
    Version,
}

impl Variant {
    pub fn from_path(path: &str) -> Option<Self> {
        match path {
            "wallpaper" => Some(Self::Full),
            "wallpaper/thumbnail" => Some(Self::Thumbnail),
            "wallpaper/version" => Some(Self::Version),
            _ => None,
        }
    }
}

/// Longer side of the swatch thumbnail; the swatch is a few dozen points wide.
const THUMBNAIL_SIDE: u32 = 160;

/// The last wallpaper read, kept until its path or modification date changes.
#[derive(Default)]
pub struct WallpaperCache(Mutex<Option<Arc<Cached>>>);

pub struct Cached {
    path: PathBuf,
    modified: SystemTime,
    kind: &'static str,
    bytes: Arc<Vec<u8>>,
    /// Opaque to the page: it only compares it, and never learns the path.
    version: String,
    thumbnail: OnceLock<Result<Arc<Vec<u8>>, String>>,
}

/// The served body, its type and the version it belongs to.
pub type Served = (Vec<u8>, &'static str, String);

/// Serves one variant of the current wallpaper. Errors are user-facing.
pub fn serve(app: &AppHandle, variant: Variant) -> Result<Served, String> {
    let path = current_path(app)?;
    let cached = load(&app.state::<WallpaperCache>().0, path)?;
    let version = cached.version.clone();
    match variant {
        Variant::Full => Ok((cached.bytes.to_vec(), cached.kind, version)),
        Variant::Version => Ok((version.clone().into_bytes(), "text/plain; charset=utf-8", version)),
        Variant::Thumbnail => Ok((thumbnail(&cached)?.to_vec(), "image/png", version)),
    }
}

/// The cached wallpaper if `path` is the same file, unchanged since it was read; otherwise
/// the file is read again and replaces it.
fn load(cache: &Mutex<Option<Arc<Cached>>>, path: PathBuf) -> Result<Arc<Cached>, String> {
    let kind = content_type(&path).ok_or("macOS no da el fondo de escritorio como una imagen (¿es dinámico o rotativo?)")?;
    let unreadable = |error: std::io::Error| format!("no se pudo leer el fondo de escritorio: {error}");
    let modified = std::fs::metadata(&path).and_then(|metadata| metadata.modified()).map_err(unreadable)?;
    let mut cache = cache.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(cached) = cache.as_ref().filter(|cached| cached.path == path && cached.modified == modified) {
        return Ok(cached.clone());
    }
    let bytes = std::fs::read(&path).map_err(unreadable)?;
    let mut hasher = DefaultHasher::new();
    (&path, modified).hash(&mut hasher);
    let cached = Arc::new(Cached {
        version: format!("{:016x}", hasher.finish()),
        path,
        modified,
        kind,
        bytes: Arc::new(bytes),
        thumbnail: OnceLock::new(),
    });
    *cache = Some(cached.clone());
    Ok(cached)
}

/// Made once per wallpaper. ImageIO reads HEIC, which the `image` crate cannot.
fn thumbnail(cached: &Cached) -> Result<Arc<Vec<u8>>, String> {
    cached
        .thumbnail
        .get_or_init(|| {
            #[cfg(target_os = "macos")]
            return crate::image_io::thumbnail_png(&cached.bytes, THUMBNAIL_SIDE).map(Arc::new);
            #[cfg(not(target_os = "macos"))]
            Ok(cached.bytes.clone())
        })
        .clone()
}

/// AppKit's screen list only exists on the main thread, while `capture://` is served off it.
fn current_path(app: &AppHandle) -> Result<PathBuf, String> {
    let (sender, receiver) = std::sync::mpsc::channel();
    app.run_on_main_thread(move || {
        let _ = sender.send(desktop_image_path());
    })
    .map_err(|error| error.to_string())?;
    receiver.recv_timeout(MAIN_THREAD_TIMEOUT).map_err(|_| "macOS no respondió a tiempo".to_string())?
}

/// The first screen is the one with the menu bar, which macOS calls the main display.
#[cfg(target_os = "macos")]
fn desktop_image_path() -> Result<PathBuf, String> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSScreen, NSWorkspace};

    let main_thread = MainThreadMarker::new().ok_or("not on the main thread")?;
    let screen = NSScreen::screens(main_thread).firstObject().ok_or("no hay ninguna pantalla")?;
    let url = NSWorkspace::sharedWorkspace()
        .desktopImageURLForScreen(&screen)
        .ok_or("macOS no da el fondo de escritorio")?;
    let path = url.path().ok_or("el fondo de escritorio no es un fichero")?;
    Ok(PathBuf::from(path.to_string()))
}

#[cfg(not(target_os = "macos"))]
fn desktop_image_path() -> Result<PathBuf, String> {
    Err("the desktop picture is only read on macOS".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serves_decodable_images_by_extension() {
        assert_eq!(content_type(Path::new("/System/Library/Desktop Pictures/Sonoma.heic")), Some("image/heic"));
        assert_eq!(content_type(Path::new("/Users/x/Fondo.JPG")), Some("image/jpeg"));
        assert_eq!(content_type(Path::new("/Users/x/fondo.png")), Some("image/png"));
    }

    #[test]
    fn variants_have_fixed_paths_only() {
        assert_eq!(Variant::from_path("wallpaper"), Some(Variant::Full));
        assert_eq!(Variant::from_path("wallpaper/thumbnail"), Some(Variant::Thumbnail));
        assert_eq!(Variant::from_path("wallpaper/version"), Some(Variant::Version));
        assert_eq!(Variant::from_path("wallpaper/../etc/passwd"), None);
    }

    #[test]
    fn the_file_is_read_again_only_when_it_changes() {
        let dir = std::env::temp_dir().join(format!("snapclone-wallpaper-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("fondo.png");
        std::fs::write(&path, b"first").unwrap();
        let cache = Mutex::default();
        let first = load(&cache, path.clone()).unwrap();
        let again = load(&cache, path.clone()).unwrap();
        assert!(Arc::ptr_eq(&first, &again), "an unchanged file is served from memory");

        std::fs::write(&path, b"second").unwrap();
        let later = SystemTime::now() + Duration::from_secs(60);
        std::fs::File::options().write(true).open(&path).unwrap().set_modified(later).unwrap();
        let changed = load(&cache, path.clone()).unwrap();
        assert_eq!(changed.bytes.as_slice(), b"second");
        assert_ne!(changed.version, first.version, "the page sees a new version");

        let other = dir.join("otro.png");
        std::fs::write(&other, b"other").unwrap();
        assert_eq!(load(&cache, other).unwrap().bytes.as_slice(), b"other", "a new path is read");
        assert!(load(&cache, dir.join("carpeta")).is_err(), "no extension, not an image");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn folders_and_unknown_formats_are_not_served() {
        assert_eq!(content_type(Path::new("/System/Library/Desktop Pictures/.thumbnails")), None);
        assert_eq!(content_type(Path::new("/Users/x/Fotos")), None);
        assert_eq!(content_type(Path::new("/Users/x/fondo.madesktop")), None);
    }
}
