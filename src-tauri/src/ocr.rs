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

/// Side of the blank image that warms Vision up; any image loads the recognizer.
#[cfg(target_os = "macos")]
const WARM_UP_SIDE: u32 = 64;

/// Loads Vision's text recognizer in the background at launch. macOS caches it per
/// executable, so the first recognition after installing or updating the app takes about
/// 30 s, and later ones a fraction of a second. Without this, that first one was the
/// automatic redaction of a "Capturar zona y copiar", which copied half a minute late.
#[cfg(target_os = "macos")]
pub fn warm_up(app: &AppHandle) {
    let Ok(dir) = app.path().app_cache_dir() else { return };
    tauri::async_runtime::spawn_blocking(move || {
        let path = dir.join("vision-warm-up.png");
        let blank = xcap::image::RgbaImage::from_pixel(WARM_UP_SIDE, WARM_UP_SIDE, xcap::image::Rgba([255, 255, 255, 255]));
        let warmed = std::fs::create_dir_all(&dir)
            .map_err(|error| error.to_string())
            .and_then(|()| crate::images::write_png(&path, &blank))
            .and_then(|()| vision::recognize(&path).map(|_| ()));
        let _ = std::fs::remove_file(&path);
        if let Err(error) = warmed {
            eprintln!("text recognition could not be warmed up: {error}");
        }
    });
}

/// Windows OCR has no first-run cost to hide, so nothing is warmed up there.
#[cfg(not(target_os = "macos"))]
pub fn warm_up(_app: &AppHandle) {}

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

#[cfg(windows)]
mod vision {
    use std::{ops::Range, path::Path};

    use windows::{
        Foundation::Rect,
        Graphics::Imaging::{
            BitmapAlphaMode, BitmapBounds, BitmapDecoder, BitmapInterpolationMode, BitmapPixelFormat, BitmapTransform,
            ColorManagementMode, ExifOrientationMode, SoftwareBitmap,
        },
        Media::Ocr::OcrEngine,
        Storage::Streams::{DataWriter, InMemoryRandomAccessStream},
    };

    use super::{NormalizedRect, Recognized, TextLine};
    use crate::redact::{Category, SensitiveRegion, find_sensitive};

    /// No real line of text is this tall, even in a HiDPI capture — the overlap between bands
    /// must comfortably exceed it, so a line split by a seam is always whole in the neighbour.
    const BAND_OVERLAP: u32 = 512;

    /// A slice of the long axis, plus the sub-range it alone owns (see `keep_line`).
    struct Band {
        start: u32,
        end: u32,
        core_start: u32,
        core_end: u32,
    }

    /// Splits `[0, total)` into bands of at most `max`, each overlapping its neighbours by
    /// `overlap`. The cores tile `[0, total)` with no gaps and no double coverage.
    fn bands(total: u32, max: u32, overlap: u32) -> Vec<Band> {
        if total <= max {
            return vec![Band { start: 0, end: total, core_start: 0, core_end: total }];
        }
        let half = overlap / 2;
        let step = max - overlap;
        let mut result = Vec::new();
        let mut start = 0u32;
        loop {
            let end = (start + max).min(total);
            let core_start = if start == 0 { 0 } else { start + half };
            let core_end = if end == total { total } else { end - half };
            result.push(Band { start, end, core_start, core_end });
            if end == total {
                break;
            }
            start += step;
        }
        result
    }

    /// Whether a line (`top`/`bottom` in full-image coordinates) belongs to `band`: its centre
    /// must fall in the band's own core, and it must not touch a seam shared with a neighbour —
    /// such a line is a fragment, read wrong, so only the neighbour that has it whole keeps it.
    fn keep_line(band: &Band, top: u32, bottom: u32, total: u32) -> bool {
        let center = (top + bottom) / 2;
        let in_core = center >= band.core_start && center < band.core_end;
        let touches_top_seam = band.start > 0 && top <= band.start;
        let touches_bottom_seam = band.end < total && bottom >= band.end;
        in_core && !touches_top_seam && !touches_bottom_seam
    }

    fn ocr_error(error: windows::core::Error) -> String {
        eprintln!("Windows OCR failed: {error}");
        "No se pudo leer el texto de la captura".into()
    }

    /// Decodes the PNG fresh from its bytes. Cheap relative to the OCR pass itself, and simpler
    /// than reusing one decoder across several crops with uncertain statefulness.
    fn decode(png: &[u8]) -> Result<BitmapDecoder, String> {
        let stream = InMemoryRandomAccessStream::new().map_err(ocr_error)?;
        let writer = DataWriter::CreateDataWriter(&stream).map_err(ocr_error)?;
        writer.WriteBytes(png).map_err(ocr_error)?;
        writer.StoreAsync().map_err(ocr_error)?.join().map_err(ocr_error)?;
        stream.Seek(0).map_err(ocr_error)?;
        BitmapDecoder::CreateAsync(&stream).map_err(ocr_error)?.join().map_err(ocr_error)
    }

    /// `bounds` crops; `scaled`, when given, additionally resizes that crop (used only for the
    /// whole-image rescale, never together with banding — see `recognize`).
    fn software_bitmap(decoder: &BitmapDecoder, bounds: BitmapBounds, scaled: Option<(u32, u32)>) -> Result<SoftwareBitmap, String> {
        let transform = BitmapTransform::new().map_err(ocr_error)?;
        transform.SetBounds(bounds).map_err(ocr_error)?;
        if let Some((width, height)) = scaled {
            transform.SetScaledWidth(width).map_err(ocr_error)?;
            transform.SetScaledHeight(height).map_err(ocr_error)?;
            transform.SetInterpolationMode(BitmapInterpolationMode::Fant).map_err(ocr_error)?;
        }
        decoder
            .GetSoftwareBitmapTransformedAsync(
                BitmapPixelFormat::Bgra8,
                BitmapAlphaMode::Premultiplied,
                &transform,
                ExifOrientationMode::IgnoreExifOrientation,
                ColorManagementMode::DoNotColorManage,
            )
            .map_err(ocr_error)?
            .join()
            .map_err(ocr_error)
    }

    /// One recognized line: text reconstructed from its words (joined with spaces — the same
    /// string `OcrLine.Text()` gives for Latin text), its union box, and the union box of the
    /// words overlapping each sensitive match. Everything here is in the bitmap that was
    /// actually analyzed (a crop, and/or a rescale), never the full image yet.
    struct Line {
        text: String,
        rect: Rect,
        sensitive: Vec<(Category, Rect)>,
    }

    fn union_rects(mut rects: impl Iterator<Item = Rect>) -> Option<Rect> {
        let first = rects.next()?;
        let (mut min_x, mut min_y) = (first.X, first.Y);
        let (mut max_x, mut max_y) = (first.X + first.Width, first.Y + first.Height);
        for rect in rects {
            min_x = min_x.min(rect.X);
            min_y = min_y.min(rect.Y);
            max_x = max_x.max(rect.X + rect.Width);
            max_y = max_y.max(rect.Y + rect.Height);
        }
        Some(Rect { X: min_x, Y: min_y, Width: max_x - min_x, Height: max_y - min_y })
    }

    fn ranges_overlap(a: &Range<usize>, b: &Range<usize>) -> bool {
        a.start < b.end && b.start < a.end
    }

    /// `words` is never empty — `ocr_lines` only calls this for lines that have at least one.
    fn process_line(words: &[(String, Rect)]) -> Line {
        let mut text = String::new();
        let mut ranges = Vec::with_capacity(words.len());
        for (word, _) in words {
            let start = text.len();
            text.push_str(word);
            ranges.push(start..text.len());
            text.push(' ');
        }
        text.pop();
        let rect = union_rects(words.iter().map(|(_, rect)| *rect)).expect("words is not empty");
        let sensitive = find_sensitive(&text)
            .into_iter()
            .filter_map(|(category, matched)| {
                let overlapping = words.iter().zip(&ranges).filter(|(_, range)| ranges_overlap(range, &matched)).map(|(&(_, rect), _)| rect);
                union_rects(overlapping).map(|rect| (category, rect))
            })
            .collect();
        Line { text, rect, sensitive }
    }

    /// Runs OCR on one already-cropped/rescaled bitmap and reconstructs its lines.
    fn ocr_lines(engine: &OcrEngine, bitmap: &SoftwareBitmap) -> Result<Vec<Line>, String> {
        let result = engine.RecognizeAsync(bitmap).map_err(ocr_error)?.join().map_err(ocr_error)?;
        let mut lines = Vec::new();
        for line in &result.Lines().map_err(ocr_error)? {
            let mut words = Vec::new();
            for word in &line.Words().map_err(ocr_error)? {
                words.push((word.Text().map_err(ocr_error)?.to_string(), word.BoundingRect().map_err(ocr_error)?));
            }
            if !words.is_empty() {
                lines.push(process_line(&words));
            }
        }
        Ok(lines)
    }

    /// Converts one already-accepted `Line` (bitmap-local pixel coordinates) into the capture's
    /// normalized ones: `offset_*` places a band's crop back in the full image (0 outside
    /// banding); `denom_*` is the pixel size that `rect` is relative to (the full image for a
    /// band — crops never rescale — or the analyzed bitmap's own size otherwise).
    fn push_line(recognized: &mut Recognized, line: Line, offset_x: f64, offset_y: f64, denom_width: f64, denom_height: f64) {
        let normalize = |rect: Rect| NormalizedRect {
            x: (f64::from(rect.X) + offset_x) / denom_width,
            y: (f64::from(rect.Y) + offset_y) / denom_height,
            width: f64::from(rect.Width) / denom_width,
            height: f64::from(rect.Height) / denom_height,
        };
        for (category, rect) in line.sensitive {
            recognized.sensitive.push(SensitiveRegion::new(category, normalize(rect)));
        }
        let rect = normalize(line.rect);
        recognized.lines.push(TextLine { text: line.text, rect });
    }

    pub fn recognize(path: &Path) -> Result<Recognized, String> {
        let png = std::fs::read(path).map_err(|error| format!("No se pudo leer la captura: {error}"))?;
        // `TryCreateFromUserProfileLanguages` returns this exact sentinel error (a null engine
        // pointer with no real HRESULT, confirmed by reading `windows-core`'s `Type::from_abi`)
        // specifically when no OCR language pack is installed. Any OTHER error — COM not
        // initialized, RPC failure, etc. — is a real failure and must not be masked by that
        // message, or a genuine bug would always look like a missing language pack.
        let engine = OcrEngine::TryCreateFromUserProfileLanguages().map_err(|error| {
            if error == windows::core::Error::empty() {
                "Windows no tiene instalado el reconocimiento de texto para tu idioma. Añádelo en \
                 Configuración › Hora e idioma › Idioma y región › (tu idioma) › Opciones de idioma."
                    .to_string()
            } else {
                ocr_error(error)
            }
        })?;
        let max = OcrEngine::MaxImageDimension().map_err(ocr_error)?;

        let decoder = decode(&png)?;
        let width = decoder.PixelWidth().map_err(ocr_error)?;
        let height = decoder.PixelHeight().map_err(ocr_error)?;
        let long = width.max(height);
        let short = width.min(height);

        let mut recognized = Recognized { lines: Vec::new(), sensitive: Vec::new() };
        // Banding only preserves whole lines when the split runs along height: a line's height
        // is bounded by its font size, comfortably inside `BAND_OVERLAP`, but its WIDTH is not —
        // a long sentence, or a card number, could straddle a vertical seam with neither half
        // recognized whole, which would silently defeat auto-redaction (a real security gap,
        // not just a quality one). So a wide image (width the long side) is always rescaled,
        // same as "both sides too large" below, even though that costs some small-text legibility.
        let along_width = width >= height;
        if long <= max || short > max || along_width {
            let scaled = (long > max).then(|| {
                let scale = f64::from(max) / f64::from(long);
                (((f64::from(width) * scale).round() as u32).max(1), ((f64::from(height) * scale).round() as u32).max(1))
            });
            let (analyzed_width, analyzed_height) = scaled.unwrap_or((width, height));
            let bounds = BitmapBounds { X: 0, Y: 0, Width: width, Height: height };
            let bitmap = software_bitmap(&decoder, bounds, scaled)?;
            for line in ocr_lines(&engine, &bitmap)? {
                push_line(&mut recognized, line, 0.0, 0.0, f64::from(analyzed_width), f64::from(analyzed_height));
            }
        } else {
            // Only height (the long side) is too large: band across it, full width, never
            // rescaled, so small text stays legible (see Key Insights in the plan).
            for band in bands(long, max, BAND_OVERLAP) {
                let bounds = BitmapBounds { X: 0, Y: band.start, Width: width, Height: band.end - band.start };
                let bitmap = software_bitmap(&decode(&png)?, bounds, None)?;
                for line in ocr_lines(&engine, &bitmap)? {
                    let top = band.start + line.rect.Y.round().max(0.0) as u32;
                    let bottom = band.start + (line.rect.Y + line.rect.Height).round().max(0.0) as u32;
                    if !keep_line(&band, top, bottom, long) {
                        continue;
                    }
                    push_line(&mut recognized, line, 0.0, f64::from(band.start), f64::from(width), f64::from(height));
                }
            }
        }
        Ok(recognized)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn tiles_without_gaps_or_overlaps(total: u32, max: u32, overlap: u32) {
            let bands = bands(total, max, overlap);
            assert_eq!(bands[0].core_start, 0, "first core starts at 0 ({total}, {max})");
            assert_eq!(bands.last().unwrap().core_end, total, "last core ends at total ({total}, {max})");
            for pair in bands.windows(2) {
                assert_eq!(pair[0].core_end, pair[1].core_start, "cores tile with no gap or overlap ({total}, {max})");
            }
        }

        #[test]
        fn band_cores_tile_the_image_with_no_gaps_or_overlaps() {
            tiles_without_gaps_or_overlaps(30_000, 10_000, 512);
            tiles_without_gaps_or_overlaps(11_520, 10_000, 512);
            tiles_without_gaps_or_overlaps(10_001, 10_000, 512);
            tiles_without_gaps_or_overlaps(9_999, 10_000, 512); // single band, no splitting needed
        }

        #[test]
        fn a_line_fully_inside_the_overlap_is_kept_by_exactly_one_band() {
            let found = bands(30_000, 10_000, 512)
                .iter()
                .filter(|band| keep_line(band, 9_700, 9_900, 30_000)) // inside the first seam's overlap
                .count();
            assert_eq!(found, 1);
        }

        #[test]
        fn a_line_crossing_a_cut_is_kept_whole_by_the_next_band_only() {
            let all = bands(30_000, 10_000, 512);
            let seam = all[0].end; // 10_000: the cut between band 0 and band 1
            let (top, bottom) = (seam - 20, seam + 20); // straddles the cut
            assert!(!keep_line(&all[0], top, bottom, 30_000), "band 0 sees it touch its bottom seam");
            assert!(keep_line(&all[1], top, bottom, 30_000), "band 1 has it whole");
            let kept = all.iter().filter(|band| keep_line(band, top, bottom, 30_000)).count();
            assert_eq!(kept, 1, "appears exactly once across all bands");
        }

        #[test]
        fn a_line_touching_the_real_image_edge_is_kept() {
            let all = bands(30_000, 10_000, 512);
            assert!(keep_line(&all[0], 0, 30, 30_000), "touches the top of the image, not a seam");
            let last = all.last().unwrap();
            assert!(keep_line(last, 29_980, 30_000, 30_000), "touches the bottom of the image, not a seam");
        }

        #[test]
        fn a_line_almost_as_tall_as_the_overlap_right_on_a_seam_still_appears_once() {
            // Exactly as tall as the overlap AND perfectly centred on the crop seam (not the
            // core boundary) touches BOTH neighbours' outer crop edges at once — the one
            // coincidence `keep_line` can't resolve, excluded by design ("el solape debe
            // superar CON MARGEN la línea más alta esperable"): a real line is never expected
            // to be exactly that tall. One pixel short of it, which any real line still is, is
            // enough to clear one of the two edges and be kept by exactly one band.
            let all = bands(30_000, 10_000, 512);
            let seam = all[0].end;
            let kept = all.iter().filter(|band| keep_line(band, seam - 511, seam, 30_000)).count();
            assert_eq!(kept, 1);
        }

        #[test]
        fn process_line_joins_words_and_finds_sensitive_overlap() {
            let words = [
                ("Contacto:".to_string(), Rect { X: 0.0, Y: 0.0, Width: 80.0, Height: 20.0 }),
                ("a@b.com".to_string(), Rect { X: 85.0, Y: 0.0, Width: 60.0, Height: 20.0 }),
            ];
            let line = process_line(&words);
            assert_eq!(line.text, "Contacto: a@b.com");
            assert_eq!(line.rect, Rect { X: 0.0, Y: 0.0, Width: 145.0, Height: 20.0 });
            assert_eq!(line.sensitive, vec![(Category::Email, Rect { X: 85.0, Y: 0.0, Width: 60.0, Height: 20.0 })]);
        }
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
mod vision {
    pub fn recognize(_path: &std::path::Path) -> Result<super::Recognized, String> {
        Err("text recognition is only implemented on macOS and Windows".into())
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
