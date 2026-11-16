//! Scroll capture. Once the selector has picked a zone, its overlays close and a thin frame
//! marks the zone instead: one transparent window just around it that lets the mouse
//! through, so the app underneath keeps scrolling. A small toolbar floats next to the zone.
//! Neither takes the focus, and Snapclone steps back so the keyboard stays with the app
//! being scrolled. A worker thread captures the zone every `SAMPLE_INTERVAL`, only with the
//! windows below the frame (so neither of ours ever shows), and stitches each frame onto the
//! previous ones; finishing hands the tall image to the normal flow.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
#[cfg(target_os = "macos")]
use std::sync::mpsc;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::{
    capture, capture_below,
    geometry::{self, Rect},
    stitch::{self, Outcome, Stitcher},
};

const WINDOW_PREFIX: &str = "scroll-";
const FRAME_WINDOW: &str = "scroll-frame";
const TOOLBAR_WINDOW: &str = "scroll-toolbar";
const PAGE: &str = "scroll.html";
const PROGRESS_EVENT: &str = "scroll-progress";
/// Frame thickness in points; drawn outside the zone, and never captured anyway.
const EDGE: u32 = 2;
const TOOLBAR_WIDTH: u32 = 460;
const TOOLBAR_HEIGHT: u32 = 56;
const TOOLBAR_GAP: u32 = 8;
const SAMPLE_INTERVAL: Duration = Duration::from_millis(150);
/// Lets the selector overlays disappear before the first frame is taken.
const SETTLE: Duration = Duration::from_millis(200);

/// The scroll capture in progress, if any. Each one gets a new id, so a worker that is
/// still winding down after a cancel never reports into the next capture's toolbar.
#[derive(Default)]
pub struct ScrollCapture {
    session: Mutex<Option<Session>>,
    next_id: AtomicU64,
}

struct Session {
    id: u64,
    stop: Arc<AtomicBool>,
    worker: JoinHandle<Result<Stitcher, String>>,
    /// Zone width in points, to work out the pixels per point of the result on macOS.
    zone_width: u32,
    /// Image pixels per CSS point of the zone's monitor — the same value `finish_scroll` needs
    /// as the result's scale, already computed once by the selector. Used directly on Windows
    /// (see the comment on `finish_scroll`); kept for parity even though macOS still recomputes
    /// its own from `zone_width`.
    scale: f64,
}

/// Where to capture: a zone in points relative to its monitor, plus every monitor's global
/// rectangle in points so the toolbar can be placed.
pub struct Target {
    pub monitor: Rect,
    pub zone: Rect,
    pub others: Vec<Rect>,
    /// Image pixels per CSS point of the zone's monitor (`MonitorShot::scale`) — needed on
    /// Windows to convert `EDGE`/`TOOLBAR_*` (CSS-point-equivalent constants) into the physical
    /// pixels `framed`/`toolbar` are built in, and to record the scroll capture's own scale.
    pub scale: f64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "lowercase")]
enum State {
    Capturing,
    /// The last frame could not be placed: the user scrolled too fast or the content changed.
    Lost,
    Full,
    Failed,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Progress {
    height: u32,
    max_height: u32,
    state: State,
}

/// Opens the frame and the toolbar and starts capturing. On error nothing is left open and
/// the caller still owns `in_progress`; on success this module releases it when done.
pub fn begin(app: &AppHandle, target: Target) -> Result<(), String> {
    let Target { monitor, zone, others, scale } = target;
    let global = Rect::new(monitor.x + zone.x, monitor.y + zone.y, zone.width, zone.height);
    let edge = native_units(EDGE, scale);
    let framed = Rect::new(global.x - edge as i32, global.y - edge as i32, global.width + edge * 2, global.height + edge * 2);
    let toolbar_size = (native_units(TOOLBAR_WIDTH, scale), native_units(TOOLBAR_HEIGHT, scale), native_units(TOOLBAR_GAP, scale));
    let toolbar = geometry::toolbar_position(framed, monitor, &others, toolbar_size.0, toolbar_size.1, toolbar_size.2);
    let below = match open_windows(app, framed, toolbar) {
        Ok(window_number) => window_number,
        Err(error) => {
            close_windows(app);
            return Err(error);
        }
    };

    let state = app.state::<ScrollCapture>();
    let id = state.next_id.fetch_add(1, Ordering::SeqCst);
    let stop = Arc::new(AtomicBool::new(false));
    // Held while the session is stored, so the worker's first report finds it in place.
    let mut session = state.session.lock().unwrap();
    let worker = std::thread::spawn({
        let (app, stop) = (app.clone(), Arc::clone(&stop));
        move || {
            let result = run(&app, id, global, below, &stop);
            if let Err(error) = &result {
                eprintln!("scroll capture stopped: {error}");
                report(&app, id, Progress { height: 0, max_height: stitch::MAX_HEIGHT, state: State::Failed });
            }
            result
        }
    });
    *session = Some(Session { id, stop, worker, zone_width: zone.width, scale });
    Ok(())
}

/// `value` is a CSS-point-equivalent constant (`EDGE`, `TOOLBAR_*`). On macOS it is used as-is:
/// the builder positions `FRAME_WINDOW`/`TOOLBAR_WINDOW` with the logical methods, which convert
/// points to physical pixels itself. On Windows `framed`/`toolbar` are built directly in
/// physical pixels (`place_physical`), so `value` must be scaled by the zone's monitor DPI first
/// — otherwise a "2-point" border/a "460-point" toolbar would occupy far fewer physical pixels
/// than the page's own CSS (which WebView2 DOES scale by the real DPI) draws them at: at 150%
/// the frame's 2px CSS border alone renders 1px wider than the window's own margin, so it bleeds
/// into the captured zone, and the toolbar's content is cramped into 2/3 of its design size.
fn native_units(value: u32, scale: f64) -> u32 {
    if cfg!(windows) { ((f64::from(value) * scale).round() as u32).max(1) } else { value }
}

/// Opens the frame and the toolbar without focusing either, and returns the frame's window
/// number: frames are captured below it.
fn open_windows(app: &AppHandle, framed: Rect, toolbar: Rect) -> Result<u32, String> {
    let frame = builder(app, FRAME_WINDOW, framed).transparent(true).build().map_err(|error| error.to_string())?;
    #[cfg(not(target_os = "macos"))]
    crate::selector::place_physical(&frame, framed.x, framed.y, framed.width, framed.height).map_err(|error| error.to_string())?;
    frame.set_ignore_cursor_events(true).map_err(|error| error.to_string())?;
    // A click on a button still works without the toolbar ever becoming the key window.
    let bar = builder(app, TOOLBAR_WINDOW, toolbar).accept_first_mouse(true).build().map_err(|error| error.to_string())?;
    #[cfg(not(target_os = "macos"))]
    crate::selector::place_physical(&bar, toolbar.x, toolbar.y, toolbar.width, toolbar.height).map_err(|error| error.to_string())?;
    #[cfg(windows)]
    for window in [&frame, &bar] {
        crate::webview_keys::disable_browser_accelerator_keys(window);
    }
    arrange(app, frame, bar)
}

/// `rect` is this window's native-unit geometry (points on macOS, physical pixels on Windows
/// — see the HiDPI note in the port plan). On macOS the logical builder methods below position
/// it directly, exactly as before. On Windows they are skipped — `open_windows` repositions
/// the built window in physical pixels instead via `selector::place_physical`, which bypasses
/// logical/DPI conversion entirely (needed because per-monitor DPI does not compose into one
/// logical space across monitors); the window stays hidden (`focused(false)`, never shown
/// before that call) so nothing flashes at the wrong spot in between.
fn builder<'a>(app: &'a AppHandle, label: &str, rect: Rect) -> WebviewWindowBuilder<'a, tauri::Wry, AppHandle> {
    // Only read on macOS, where the builder positions the window directly; `open_windows`
    // repositions it afterwards on every other platform.
    let _ = rect;
    #[allow(unused_mut)]
    let mut builder = WebviewWindowBuilder::new(app, label, WebviewUrl::App(PAGE.into()))
        .title("Snapclone")
        .decorations(false)
        .resizable(false)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible_on_all_workspaces(true)
        .focused(false)
        .focusable(false);
    #[cfg(target_os = "macos")]
    {
        builder = builder.position(f64::from(rect.x), f64::from(rect.y)).inner_size(f64::from(rect.width), f64::from(rect.height));
    }
    builder
}

/// Puts the frame above every app's windows and the toolbar above the frame, so capturing
/// below the frame leaves both out. Then Snapclone steps back: the selector it comes from
/// had the keyboard, and the app being scrolled should get it back.
#[cfg(target_os = "macos")]
fn arrange(app: &AppHandle, frame: WebviewWindow, toolbar: WebviewWindow) -> Result<u32, String> {
    let (sender, receiver) = mpsc::channel();
    app.run_on_main_thread(move || {
        use objc2::MainThreadMarker;
        use objc2_app_kit::{NSApplication, NSPopUpMenuWindowLevel, NSWindow};
        let result = (|| {
            let frame = frame.ns_window().map_err(|error| error.to_string())?;
            let toolbar = toolbar.ns_window().map_err(|error| error.to_string())?;
            // SAFETY: Tauri hands out the live NSWindows of these windows, and AppKit is only
            // touched here on the main thread.
            let (frame, toolbar) = unsafe { (&*frame.cast::<NSWindow>(), &*toolbar.cast::<NSWindow>()) };
            frame.setLevel(NSPopUpMenuWindowLevel);
            toolbar.setLevel(NSPopUpMenuWindowLevel + 1);
            if let Some(main_thread) = MainThreadMarker::new() {
                NSApplication::sharedApplication(main_thread).deactivate();
            }
            u32::try_from(frame.windowNumber()).map_err(|_| "the frame has no window number".to_string())
        })();
        let _ = sender.send(result);
    })
    .map_err(|error| error.to_string())?;
    receiver.recv().map_err(|error| error.to_string())?
}

/// Phase 2 spike (step 1a): the frame sits `EDGE` px outside the zone and the toolbar beside
/// it, never over it, so a plain region capture below already leaves both out in the common
/// case — no window-level content-protection trick needed here, unlike macOS's window levels.
/// `below` has no Windows equivalent (see `capture_below_other::capture`), so `0` is returned
/// and ignored by the caller.
#[cfg(target_os = "windows")]
fn arrange(_app: &AppHandle, _frame: WebviewWindow, _toolbar: WebviewWindow) -> Result<u32, String> {
    Ok(0)
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn arrange(_app: &AppHandle, _frame: WebviewWindow, _toolbar: WebviewWindow) -> Result<u32, String> {
    Err("scroll capture is only available on macOS and Windows".into())
}

fn close_windows(app: &AppHandle) {
    for (label, window) in app.webview_windows() {
        if label.starts_with(WINDOW_PREFIX) {
            let _ = window.destroy();
        }
    }
}

/// Sends progress to the toolbar, but only while `id` is still the current capture.
fn report(app: &AppHandle, id: u64, progress: Progress) {
    let state = app.state::<ScrollCapture>();
    let session = state.session.lock().unwrap();
    if session.as_ref().is_none_or(|session| session.id != id) {
        return;
    }
    if let Err(error) = app.emit_to(TOOLBAR_WINDOW, PROGRESS_EVENT, progress) {
        eprintln!("scroll-progress emit failed: {error}");
    }
}

/// The capture loop. Sampling keeps a steady pace without piling up: a slow capture just
/// delays the next one.
fn run(app: &AppHandle, id: u64, zone: Rect, below: u32, stop: &AtomicBool) -> Result<Stitcher, String> {
    std::thread::sleep(SETTLE);
    let grab = || capture_below::capture(zone, below);
    let mut stitcher = Stitcher::new(&grab()?);
    let mut state = State::Capturing;
    report(app, id, Progress { height: stitcher.height(), max_height: stitch::MAX_HEIGHT, state: state.clone() });
    while !stop.load(Ordering::SeqCst) {
        let started = Instant::now();
        match grab() {
            Ok(frame) => {
                state = match stitcher.push(&frame) {
                    Outcome::Appended(_) => State::Capturing,
                    Outcome::Unchanged => state,
                    Outcome::NoOverlap => State::Lost,
                    Outcome::LimitReached => State::Full,
                }
            }
            // A single failed frame (a display reconfiguring) is not worth ending the capture.
            Err(error) => eprintln!("scroll frame could not be captured: {error}"),
        }
        report(app, id, Progress { height: stitcher.height(), max_height: stitch::MAX_HEIGHT, state: state.clone() });
        std::thread::sleep(SAMPLE_INTERVAL.saturating_sub(started.elapsed()));
    }
    Ok(stitcher)
}

fn take_session(app: &AppHandle) -> Option<Session> {
    let session = app.state::<ScrollCapture>().session.lock().unwrap().take();
    if let Some(session) = &session {
        session.stop.store(true, Ordering::SeqCst);
        close_windows(app);
    }
    session
}

/// Discards the capture. The worker notices the stop flag and ends by itself; it can no
/// longer report anywhere, since its session is gone.
fn abandon(app: &AppHandle) {
    if take_session(app).is_some() {
        capture::selection_ended(app);
    }
}

/// The frame or the toolbar closed some other way must not leave the other behind.
pub fn on_window_destroyed(app: &AppHandle, label: &str) {
    if label.starts_with(WINDOW_PREFIX) {
        abandon(app);
    }
}

#[tauri::command]
pub fn cancel_scroll(app: AppHandle) {
    abandon(&app);
}

/// Stops capturing and stores the stitched image like any other capture.
#[tauri::command]
pub async fn finish_scroll(app: AppHandle) -> Result<(), String> {
    let session = take_session(&app).ok_or("no scroll capture in progress")?;
    let result = async {
        let stitcher = tauri::async_runtime::spawn_blocking(move || session.worker.join())
            .await
            .map_err(|error| error.to_string())?
            .map_err(|_| "the scroll capture stopped unexpectedly".to_string())??;
        let image = stitcher.into_image();
        // On Windows `zone_width` is already physical pixels (post-HiDPI-redesign), so the
        // pixels-per-point ratio below would come out ~1.0 regardless of the real DPI — use the
        // scale the selector already computed for this monitor instead. macOS keeps the ratio
        // unchanged, since there `zone_width` is genuinely in points.
        let scale =
            if cfg!(windows) { session.scale } else { f64::from(image.width()) / f64::from(session.zone_width.max(1)) };
        capture::store_image(&app, image, scale).await
    }
    .await;
    if let Err(error) = &result {
        capture::report_failure(&app, error);
    }
    capture::selection_ended(&app);
    result
}
