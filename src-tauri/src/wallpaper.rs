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

/// Formats the editor's webview decodes directly in an `<img>`, by file extension, with the
/// type they are served as. HEIC/HEIF/TIFF render fine in Safari/WebKit (macOS) but Chromium/
/// WebView2 (Windows) has no built-in decoder for either — advertising them as servable there
/// would silently show a broken image instead of `load`'s clear "not available" fallback.
#[cfg(target_os = "macos")]
const CONTENT_TYPES: [(&str, &str); 7] = [
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("heic", "image/heic"),
    ("heif", "image/heif"),
    ("tif", "image/tiff"),
    ("tiff", "image/tiff"),
];

#[cfg(not(target_os = "macos"))]
const CONTENT_TYPES: [(&str, &str); 3] = [("png", "image/png"), ("jpg", "image/jpeg"), ("jpeg", "image/jpeg")];

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
    let cached = current(app)?;
    let version = cached.version.clone();
    match variant {
        Variant::Full => Ok((cached.bytes.to_vec(), cached.kind, version)),
        Variant::Version => Ok((version.clone().into_bytes(), "text/plain; charset=utf-8", version)),
        Variant::Thumbnail => Ok((thumbnail(&cached)?.to_vec(), "image/png", version)),
    }
}

/// The current wallpaper file, read again only when it changed. Asks AppKit on the main
/// thread and waits, so it must be called off the main thread.
pub fn current(app: &AppHandle) -> Result<Arc<Cached>, String> {
    load(&app.state::<WallpaperCache>().0, current_path(app)?)
}

// Read only by the macOS native compositor.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
impl Cached {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Derived from the path and the modification date, so it changes with either.
    pub fn version(&self) -> &str {
        &self.version
    }
}

/// The cached wallpaper if `path` is the same file, unchanged since it was read; otherwise
/// the file is read again and replaces it.
fn load(cache: &Mutex<Option<Arc<Cached>>>, path: PathBuf) -> Result<Arc<Cached>, String> {
    let kind = content_type(&path);
    #[cfg(not(target_os = "macos"))]
    let kind = kind.or_else(|| sniffed_type(&path));
    let kind = kind.ok_or_else(no_wallpaper_image_error)?;
    let unreadable = |error: std::io::Error| format!("no se pudo leer el fondo de escritorio: {error}");
    let metadata = std::fs::metadata(&path).map_err(unreadable)?;
    let modified = metadata.modified().map_err(unreadable)?;
    let mut cache = cache.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(cached) = cache.as_ref().filter(|cached| cached.path == path && cached.modified == modified) {
        return Ok(cached.clone());
    }
    // Checked from the metadata already in hand, BEFORE reading the file: a `std::fs::read`
    // of an oversized wallpaper would itself allocate and copy the whole thing just to then
    // discard it.
    #[cfg(not(target_os = "macos"))]
    if metadata.len() > crate::import::MAX_FILE_BYTES {
        return Err("el fondo de escritorio es demasiado grande para mostrarlo".into());
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

/// Neither "dynamic/rotating desktop" (macOS) nor "slideshow/Windows Spotlight" (Windows) give
/// the editor a plain image file to blur, so both say so without naming either platform in the
/// other's text.
fn no_wallpaper_image_error() -> String {
    if cfg!(target_os = "macos") {
        "macOS no da el fondo de escritorio como una imagen (¿es dinámico o rotativo?)".into()
    } else {
        "El fondo de escritorio no está disponible como imagen (¿es una presentación o un fondo de Windows Spotlight?)".into()
    }
}

/// Only for `TranscodedWallpaper`: Windows' own cached copy of the current wallpaper has no
/// extension at all, so `content_type` always misses it by name — read just enough of the
/// header to tell it is really a JPEG (which it always is here) instead of trusting the name.
#[cfg(not(target_os = "macos"))]
fn sniffed_type(path: &Path) -> Option<&'static str> {
    use std::io::Read;
    let mut header = [0u8; 3];
    std::fs::File::open(path).ok()?.read_exact(&mut header).ok()?;
    (header == [0xFF, 0xD8, 0xFF]).then_some("image/jpeg")
}

/// Made once per wallpaper. ImageIO reads HEIC, which the `image` crate cannot.
fn thumbnail(cached: &Cached) -> Result<Arc<Vec<u8>>, String> {
    cached
        .thumbnail
        .get_or_init(|| {
            #[cfg(target_os = "macos")]
            return crate::image_io::thumbnail_png(&cached.bytes, THUMBNAIL_SIDE).map(Arc::new);
            #[cfg(target_os = "windows")]
            return windows_thumbnail(&cached.bytes);
            #[cfg(not(any(target_os = "macos", target_os = "windows")))]
            Ok(cached.bytes.clone())
        })
        .clone()
}

/// The header is checked before any decoding, the same guard `import::prepare` uses for an
/// untrusted local image: the wallpaper path is user-controlled state, not something this app
/// wrote itself.
#[cfg(target_os = "windows")]
fn windows_thumbnail(bytes: &[u8]) -> Result<Arc<Vec<u8>>, String> {
    const FAILED: &str = "no se pudo reducir el fondo de escritorio";
    let (width, height) = crate::import::header_size(bytes).ok_or(FAILED)?;
    crate::import::check_size(width, height)?;
    let image = xcap::image::load_from_memory(bytes).map_err(|_| FAILED.to_string())?;
    let longest = image.width().max(image.height());
    let resized = if longest > THUMBNAIL_SIDE {
        let scale = f64::from(THUMBNAIL_SIDE) / f64::from(longest);
        let width = ((f64::from(image.width()) * scale).round() as u32).max(1);
        let height = ((f64::from(image.height()) * scale).round() as u32).max(1);
        image.resize(width, height, xcap::image::imageops::FilterType::Lanczos3)
    } else {
        image
    };
    let mut png = Vec::new();
    resized.write_to(&mut std::io::Cursor::new(&mut png), xcap::image::ImageFormat::Png).map_err(|_| FAILED.to_string())?;
    Ok(Arc::new(png))
}

/// On macOS AppKit's screen list only exists on the main thread; harmless but unnecessary on
/// Windows, where `run_on_main_thread` still works fine, so it is left as is on every platform.
fn current_path(app: &AppHandle) -> Result<PathBuf, String> {
    let (sender, receiver) = std::sync::mpsc::channel();
    app.run_on_main_thread(move || {
        let _ = sender.send(desktop_image_path());
    })
    .map_err(|error| error.to_string())?;
    receiver.recv_timeout(MAIN_THREAD_TIMEOUT).map_err(|_| timeout_error())?
}

/// Same wording macOS has always used (unchanged by the Windows port, per the project's
/// zero-behaviour-change rule there), with a Windows-specific counterpart next to it — same
/// pattern as the sibling `no_wallpaper_image_error` below.
fn timeout_error() -> String {
    if cfg!(target_os = "macos") { "macOS no respondió a tiempo".into() } else { "Windows no respondió a tiempo".into() }
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

/// `SystemParametersInfoW` first (the wallpaper the user actually set, a real file with a real
/// extension); if that is missing, not a usable extension, or the call itself fails (slideshow,
/// Spotlight, solid colour — none of which name a single static file), Windows' own rendered
/// copy of whatever is showing, `TranscodedWallpaper`, is used instead — an extensionless JPEG,
/// identified by `sniffed_type` in `load`, not by this function.
#[cfg(target_os = "windows")]
fn desktop_image_path() -> Result<PathBuf, String> {
    use windows::Win32::{
        Foundation::MAX_PATH,
        UI::WindowsAndMessaging::{SPI_GETDESKWALLPAPER, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW},
    };

    let mut buffer = [0u16; MAX_PATH as usize];
    // SAFETY: `buffer` holds exactly `MAX_PATH` wide characters, as the API requires.
    let result = unsafe {
        SystemParametersInfoW(SPI_GETDESKWALLPAPER, buffer.len() as u32, Some(buffer.as_mut_ptr().cast()), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0))
    };
    if result.is_ok() {
        let length = buffer.iter().position(|&unit| unit == 0).unwrap_or(buffer.len());
        let path = PathBuf::from(String::from_utf16_lossy(&buffer[..length]));
        // `path.is_file()`: Windows keeps pointing SPI_GETDESKWALLPAPER at this path even after
        // the file itself is deleted or moved away — without this check, a stale path would be
        // returned as "the" wallpaper and fail downstream instead of falling back to
        // `TranscodedWallpaper`, which is still there since it is Windows' own rendered copy.
        if !path.as_os_str().is_empty() && path.is_file() && content_type(&path).is_some() {
            return Ok(path);
        }
    }
    transcoded_wallpaper_path().ok_or_else(|| "Windows no da el fondo de escritorio".into())
}

#[cfg(target_os = "windows")]
fn transcoded_wallpaper_path() -> Option<PathBuf> {
    let path = PathBuf::from(std::env::var_os("APPDATA")?).join(r"Microsoft\Windows\Themes\TranscodedWallpaper");
    path.is_file().then_some(path)
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn desktop_image_path() -> Result<PathBuf, String> {
    Err("the desktop picture is only read on macOS and Windows".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serves_decodable_images_by_extension() {
        assert_eq!(content_type(Path::new("/Users/x/Fondo.JPG")), Some("image/jpeg"));
        assert_eq!(content_type(Path::new("/Users/x/fondo.png")), Some("image/png"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_also_serves_heic_and_tiff() {
        assert_eq!(content_type(Path::new("/System/Library/Desktop Pictures/Sonoma.heic")), Some("image/heic"));
        assert_eq!(content_type(Path::new("/Users/x/fondo.tiff")), Some("image/tiff"));
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn heic_and_tiff_are_not_served_outside_macos() {
        assert_eq!(content_type(Path::new("/Users/x/fondo.heic")), None, "Chromium/WebView2 cannot decode HEIC");
        assert_eq!(content_type(Path::new("/Users/x/fondo.tiff")), None, "Chromium/WebView2 cannot decode TIFF");
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
