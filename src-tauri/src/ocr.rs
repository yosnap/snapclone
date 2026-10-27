//! Vision text recognition, run once per capture. A single pass gives the recognized lines
//! (for "Texto detectado") and the sensitive regions within them (for automatic redaction);
//! both are cached by capture id, so neither feature analyses a capture the other already did.

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::{capture, editor_window, export, history, redact::SensitiveRegion};

/// Analyses kept in memory; a capture's lines are a few kilobytes.
const CACHE_CAPACITY: usize = 32;

/// Normalized (0..1) box with the origin at the capture's top-left corner.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct NormalizedRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TextLine {
    pub text: String,
    #[serde(flatten)]
    pub rect: NormalizedRect,
}

pub struct Analysis {
    /// In reading order: rows top to bottom, each row left to right.
    pub lines: Vec<TextLine>,
    pub sensitive: Vec<SensitiveRegion>,
}

type Slot<T> = Arc<Mutex<Option<Arc<T>>>>;

/// Least-recently-used results by key. Each key has its own slot, locked while its value is
/// computed, so two requests for the same capture wait for one analysis instead of running
/// two. Errors are not stored, so a failed analysis is tried again next time.
pub struct Cache<T> {
    slots: Mutex<VecDeque<(String, Slot<T>)>>,
    capacity: usize,
}

impl<T> Cache<T> {
    pub fn new(capacity: usize) -> Self {
        Self { slots: Mutex::new(VecDeque::new()), capacity }
    }

    /// The slot for `key`, moved to the most recent end; the oldest is dropped when full.
    fn slot(&self, key: &str) -> Slot<T> {
        let mut slots = self.slots.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(index) = slots.iter().position(|(existing, _)| existing == key) {
            let entry = slots.remove(index).expect("index was just found");
            let slot = entry.1.clone();
            slots.push_back(entry);
            return slot;
        }
        let slot = Slot::default();
        slots.push_back((key.to_string(), slot.clone()));
        if slots.len() > self.capacity {
            slots.pop_front();
        }
        slot
    }

    pub fn get_or_compute(&self, key: &str, compute: impl FnOnce() -> Result<T, String>) -> Result<Arc<T>, String> {
        let slot = self.slot(key);
        let mut value = slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(cached) = value.as_ref() {
            return Ok(cached.clone());
        }
        let computed = Arc::new(compute()?);
        *value = Some(computed.clone());
        Ok(computed)
    }

    pub fn forget(&self, key: &str) {
        let mut slots = self.slots.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        slots.retain(|(existing, _)| existing != key);
    }
}

pub struct OcrCache(Cache<Analysis>);

impl Default for OcrCache {
    fn default() -> Self {
        Self(Cache::new(CACHE_CAPACITY))
    }
}

/// Sorts lines as they are read. Vision returns them roughly top to bottom, but lines on the
/// same row (columns, a label and its value) can come in any order. A line joins the current
/// row when its vertical centre falls within the row's first line, so slightly uneven
/// baselines still read left to right.
pub fn reading_order(mut lines: Vec<TextLine>) -> Vec<TextLine> {
    lines.sort_by(|a, b| a.rect.y.total_cmp(&b.rect.y));
    let mut rows: Vec<Vec<TextLine>> = Vec::new();
    for line in lines {
        let centre = line.rect.y + line.rect.height / 2.0;
        match rows.last_mut() {
            Some(row) if centre <= row[0].rect.y + row[0].rect.height => row.push(line),
            _ => rows.push(vec![line]),
        }
    }
    rows.into_iter()
        .flat_map(|mut row| {
            row.sort_by(|a, b| a.rect.x.total_cmp(&b.rect.x));
            row
        })
        .collect()
}

/// The capture's analysis, from the cache or from Vision off the async runtime.
pub async fn analyze(app: &AppHandle, id: &str) -> Result<Arc<Analysis>, String> {
    let path = capture::path_of(app, id).ok_or("unknown capture")?;
    let app = app.clone();
    let id = id.to_string();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<OcrCache>().0.get_or_compute(&id, || {
            let analysis = vision::recognize(&path)?;
            Ok(Analysis { lines: reading_order(analysis.lines), sensitive: analysis.sensitive })
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Drops a deleted capture's analysis.
pub fn forget(app: &AppHandle, id: &str) {
    app.state::<OcrCache>().0.forget(id);
}

#[tauri::command]
pub async fn recognize_text(app: AppHandle, id: String) -> Result<Vec<TextLine>, String> {
    Ok(analyze(&app, &id).await?.lines.clone())
}

/// One line of text per recognized line, as "Copiar todo" puts it on the pasteboard.
pub fn joined(lines: &[TextLine]) -> String {
    lines.iter().map(|line| line.text.as_str()).collect::<Vec<_>>().join("\n")
}

/// Puts the capture's recognized text on the pasteboard, one line per recognized line, and
/// returns how many lines it copied. Nothing is copied when there is no text.
async fn copy_text_of(app: &AppHandle, id: &str) -> Result<usize, String> {
    let analysis = analyze(app, id).await.map_err(|error| format!("No se pudo leer el texto: {error}"))?;
    if analysis.lines.is_empty() {
        return Err("No se ha encontrado texto en esta captura".into());
    }
    let text = joined(&analysis.lines);
    let (sender, receiver) = std::sync::mpsc::channel();
    // AppKit's pasteboard is used from the main thread, like the editor's image copy.
    app.run_on_main_thread(move || {
        let _ = sender.send(export::write_text_to_pasteboard(&text));
    })
    .map_err(|error| error.to_string())?;
    tauri::async_runtime::spawn_blocking(move || receiver.recv())
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())?
        .map_err(|error| format!("No se pudo copiar el texto: {error}"))?;
    Ok(analysis.lines.len())
}

/// "Copiar todo" in the detected text view.
#[tauri::command]
pub async fn copy_capture_text(app: AppHandle, id: String) -> Result<usize, String> {
    copy_text_of(&app, &id).await
}

/// "Copiar texto de la última captura" in the menu bar icon. Errors bring the editor forward
/// with the reason; a successful copy only updates its status line, without stealing focus.
pub fn copy_latest_text(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let id = match history::latest_id(&app) {
            Ok(Some(id)) => id,
            Ok(None) => return editor_window::notify(&app, "Todavía no hay ninguna captura de la que copiar el texto", false),
            Err(error) => return editor_window::notify(&app, &format!("No se pudo leer el historial: {error}"), true),
        };
        match copy_text_of(&app, &id).await {
            Ok(count) => editor_window::status(&app, &format!("Texto de la última captura copiado ({count} líneas)")),
            Err(error) => editor_window::notify(&app, &error, true),
        }
    });
}

/// What one Vision pass finds, before the lines are put in reading order.
struct Recognized {
    lines: Vec<TextLine>,
    sensitive: Vec<SensitiveRegion>,
}

#[cfg(target_os = "macos")]
mod vision {
    use std::path::Path;

    use objc2::{AnyThread, msg_send, rc::Retained};
    use objc2_core_foundation::CGRect;
    use objc2_foundation::{NSArray, NSDictionary, NSError, NSRange, NSString, NSURL};
    use objc2_vision::{
        VNImageRequestHandler, VNRecognizeTextRequest, VNRecognizedText, VNRectangleObservation, VNRequest,
        VNRequestTextRecognitionLevel,
    };

    use super::{NormalizedRect, Recognized, TextLine};
    use crate::redact::{Category, SensitiveRegion, find_sensitive};

    pub fn recognize(path: &Path) -> Result<Recognized, String> {
        // Runs on a worker thread, so drain autoreleased Vision objects here.
        objc2::rc::autoreleasepool(|_| recognize_in_pool(path))
    }

    fn recognize_in_pool(path: &Path) -> Result<Recognized, String> {
        let path = path.to_str().ok_or("capture path is not UTF-8")?;
        let url = NSURL::fileURLWithPath(&NSString::from_str(path));
        // SAFETY: an empty options dictionary is valid for every VNImageOption key type.
        let handler = unsafe {
            VNImageRequestHandler::initWithURL_options(VNImageRequestHandler::alloc(), &url, &NSDictionary::new())
        };

        let request = VNRecognizeTextRequest::new();
        request.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
        // Correction "fixes" keys and addresses into dictionary words, which breaks matching.
        request.setUsesLanguageCorrection(false);
        let as_request: &VNRequest = &request;
        handler
            .performRequests_error(&NSArray::from_slice(&[as_request]))
            .map_err(|error| error.localizedDescription().to_string())?;

        let mut recognized = Recognized { lines: Vec::new(), sensitive: Vec::new() };
        let Some(observations) = request.results() else {
            return Ok(recognized);
        };
        for observation in observations.iter() {
            let Some(text) = observation.topCandidates(1).firstObject() else {
                continue;
            };
            let line = text.string().to_string();
            for (category, range) in find_sensitive(&line) {
                recognized.sensitive.extend(bounding_box(&text, &line, range, category));
            }
            // SAFETY: reading a plain CGRect property.
            let rect = unsafe { observation.boundingBox() };
            recognized.lines.push(TextLine { text: line, rect: normalized(rect) });
        }
        Ok(recognized)
    }

    /// Vision uses a bottom-left origin.
    fn normalized(rect: CGRect) -> NormalizedRect {
        NormalizedRect {
            x: rect.origin.x,
            y: 1.0 - rect.origin.y - rect.size.height,
            width: rect.size.width,
            height: rect.size.height,
        }
    }

    /// Vision addresses characters in UTF-16 units.
    fn bounding_box(
        text: &VNRecognizedText,
        line: &str,
        bytes: std::ops::Range<usize>,
        category: Category,
    ) -> Option<SensitiveRegion> {
        let start = line[..bytes.start].encode_utf16().count();
        let length = line[bytes].encode_utf16().count();
        // SAFETY: the range lies within the candidate string it was computed from.
        let observation: Result<Retained<VNRectangleObservation>, Retained<NSError>> =
            unsafe { msg_send![text, boundingBoxForRange: NSRange::new(start, length), error: _] };
        // SAFETY: reading a plain CGRect property.
        let rect = unsafe { observation.ok()?.boundingBox() };
        Some(SensitiveRegion::new(category, normalized(rect)))
    }
}

#[cfg(not(target_os = "macos"))]
mod vision {
    pub fn recognize(_path: &std::path::Path) -> Result<super::Recognized, String> {
        Err("text recognition is only implemented on macOS".into())
    }
}

/// Runs Vision on an image outside the app, for the manual test in `redact`.
#[cfg(test)]
pub fn recognize_sample(path: &std::path::Path) -> Analysis {
    let recognized = vision::recognize(path).expect("vision runs");
    Analysis { lines: reading_order(recognized.lines), sensitive: recognized.sensitive }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn line(text: &str, x: f64, y: f64, height: f64) -> TextLine {
        TextLine { text: text.into(), rect: NormalizedRect { x, y, width: 0.2, height } }
    }

    fn texts(lines: &[TextLine]) -> Vec<&str> {
        lines.iter().map(|line| line.text.as_str()).collect()
    }

    #[test]
    fn analyses_once_and_reuses_the_result() {
        let cache = Cache::new(4);
        let runs = AtomicUsize::new(0);
        let compute = || {
            runs.fetch_add(1, Ordering::SeqCst);
            Ok(42)
        };
        assert_eq!(*cache.get_or_compute("a", compute).unwrap(), 42);
        assert_eq!(*cache.get_or_compute("a", compute).unwrap(), 42);
        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn failures_are_not_cached() {
        let cache = Cache::new(4);
        assert!(cache.get_or_compute("a", || Err::<i32, _>("Vision failed".into())).is_err());
        assert_eq!(*cache.get_or_compute("a", || Ok(7)).unwrap(), 7);
    }

    #[test]
    fn least_recently_used_entry_is_dropped_when_full() {
        let cache = Cache::new(2);
        cache.get_or_compute("a", || Ok(1)).unwrap();
        cache.get_or_compute("b", || Ok(2)).unwrap();
        cache.get_or_compute("a", || Ok(0)).unwrap(); // touches "a", so "b" is now the oldest
        cache.get_or_compute("c", || Ok(3)).unwrap();
        assert_eq!(*cache.get_or_compute("a", || Ok(-1)).unwrap(), 1, "a was kept");
        assert_eq!(*cache.get_or_compute("b", || Ok(-2)).unwrap(), -2, "b was evicted and recomputed");
    }

    #[test]
    fn forgotten_entries_are_computed_again() {
        let cache = Cache::new(4);
        cache.get_or_compute("a", || Ok(1)).unwrap();
        cache.forget("a");
        assert_eq!(*cache.get_or_compute("a", || Ok(2)).unwrap(), 2);
    }

    #[test]
    fn concurrent_requests_for_one_capture_share_one_analysis() {
        let cache = Arc::new(Cache::new(4));
        let runs = Arc::new(AtomicUsize::new(0));
        let threads: Vec<_> = (0..4)
            .map(|_| {
                let (cache, runs) = (cache.clone(), runs.clone());
                std::thread::spawn(move || {
                    *cache
                        .get_or_compute("a", || {
                            runs.fetch_add(1, Ordering::SeqCst);
                            std::thread::sleep(std::time::Duration::from_millis(50));
                            Ok(5)
                        })
                        .unwrap()
                })
            })
            .collect();
        for thread in threads {
            assert_eq!(thread.join().unwrap(), 5);
        }
        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn lines_are_read_by_rows_then_left_to_right() {
        let lines = vec![
            line("valor", 0.5, 0.102, 0.03),
            line("pie", 0.1, 0.8, 0.03),
            line("Etiqueta", 0.1, 0.1, 0.03),
            line("Título", 0.1, 0.02, 0.04),
        ];
        assert_eq!(texts(&reading_order(lines)), ["Título", "Etiqueta", "valor", "pie"]);
    }

    #[test]
    fn close_but_separate_rows_are_not_merged() {
        let lines = vec![line("segunda", 0.0, 0.14, 0.03), line("primera", 0.6, 0.1, 0.03)];
        assert_eq!(texts(&reading_order(lines)), ["primera", "segunda"]);
    }

    #[test]
    fn copied_text_has_one_line_per_recognized_line() {
        assert_eq!(joined(&[line("uno", 0.0, 0.0, 0.1), line("dos", 0.0, 0.2, 0.1)]), "uno\ndos");
        assert_eq!(joined(&[]), "");
    }
}
