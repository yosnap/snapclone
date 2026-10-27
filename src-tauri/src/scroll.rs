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
        mpsc,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

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
    /// Zone width in points, to work out the pixels per point of the result.
    zone_width: u32,
}

/// Where to capture: a zone in points relative to its monitor, plus every monitor's global
/// rectangle in points so the toolbar can be placed.
pub struct Target {
    pub monitor: Rect,
    pub zone: Rect,
    pub others: Vec<Rect>,
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
    let Target { monitor, zone, others } = target;
    let global = Rect::new(monitor.x + zone.x, monitor.y + zone.y, zone.width, zone.height);
    let framed = Rect::new(global.x - EDGE as i32, global.y - EDGE as i32, global.width + EDGE * 2, global.height + EDGE * 2);
    let toolbar = geometry::toolbar_position(framed, monitor, &others, TOOLBAR_WIDTH, TOOLBAR_HEIGHT, TOOLBAR_GAP);
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
    *session = Some(Session { id, stop, worker, zone_width: zone.width });
    Ok(())
}

/// Opens the frame and the toolbar without focusing either, and returns the frame's window
/// number: frames are captured below it.
fn open_windows(app: &AppHandle, framed: Rect, toolbar: Rect) -> Result<u32, String> {
    let frame = builder(app, FRAME_WINDOW, framed).transparent(true).build().map_err(|error| error.to_string())?;
    frame.set_ignore_cursor_events(true).map_err(|error| error.to_string())?;
    // A click on a button still works without the toolbar ever becoming the key window.
    let bar = builder(app, TOOLBAR_WINDOW, toolbar).accept_first_mouse(true).build().map_err(|error| error.to_string())?;
    arrange(app, frame, bar)
}

fn builder<'a>(app: &'a AppHandle, label: &str, rect: Rect) -> WebviewWindowBuilder<'a, tauri::Wry, AppHandle> {
    WebviewWindowBuilder::new(app, label, WebviewUrl::App(PAGE.into()))
        .title("Snapclone")
        .position(f64::from(rect.x), f64::from(rect.y))
        .inner_size(f64::from(rect.width), f64::from(rect.height))
        .decorations(false)
        .resizable(false)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible_on_all_workspaces(true)
        .focused(false)
        .focusable(false)
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

#[cfg(not(target_os = "macos"))]
fn arrange(_app: &AppHandle, _frame: WebviewWindow, _toolbar: WebviewWindow) -> Result<u32, String> {
    Err("scroll capture is only available on macOS".into())
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
        let scale = f64::from(image.width()) / f64::from(session.zone_width.max(1));
        capture::store_image(&app, image, scale).await
    }
    .await;
    if let Err(error) = &result {
        capture::report_failure(&app, error);
    }
    capture::selection_ended(&app);
    result
}
