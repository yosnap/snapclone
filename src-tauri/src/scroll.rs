//! Scroll capture. Once the selector has picked a zone, its overlays close and a thin frame
//! marks the zone instead: four borderless windows just outside it, which let the mouse
//! through so the app underneath keeps scrolling. A small toolbar floats next to the zone,
//! never over it. A worker thread captures the zone every `SAMPLE_INTERVAL` and stitches
//! each frame onto the previous ones; finishing hands the tall image to the normal flow.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder, window::Color};

use crate::{
    capture,
    geometry::{self, Rect},
    selector,
    stitch::{self, Outcome, Stitcher},
};

const WINDOW_PREFIX: &str = "scroll-";
const TOOLBAR_WINDOW: &str = "scroll-toolbar";
const PAGE: &str = "scroll.html";
const PROGRESS_EVENT: &str = "scroll-progress";
/// Frame thickness in points; drawn outside the zone so it never shows in the capture.
const EDGE: u32 = 2;
const EDGE_COLOR: Color = Color(37, 99, 235, 255);
const TOOLBAR_WIDTH: u32 = 460;
const TOOLBAR_HEIGHT: u32 = 56;
const TOOLBAR_GAP: u32 = 8;
const SAMPLE_INTERVAL: Duration = Duration::from_millis(150);
/// Lets the selector overlays disappear before the first frame is taken.
const SETTLE: Duration = Duration::from_millis(200);

/// The scroll capture in progress, if any.
#[derive(Default)]
pub struct ScrollCapture(Mutex<Option<Session>>);

struct Session {
    stop: Arc<AtomicBool>,
    worker: JoinHandle<Result<Stitcher, String>>,
    /// Zone width in points, to work out the pixels per point of the result.
    zone_width: u32,
}

/// Where to capture: a zone in points relative to its monitor, plus every monitor's global
/// rectangle in points so the toolbar can be placed.
pub struct Target {
    pub monitor_id: u32,
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
    let Target { monitor_id, monitor, zone, others } = target;
    let global = Rect::new(monitor.x + zone.x, monitor.y + zone.y, zone.width, zone.height);
    let framed = Rect::new(global.x - EDGE as i32, global.y - EDGE as i32, global.width + EDGE * 2, global.height + EDGE * 2);
    let toolbar = geometry::toolbar_position(framed, monitor, &others, TOOLBAR_WIDTH, TOOLBAR_HEIGHT, TOOLBAR_GAP);
    if let Err(error) = open_windows(app, global, toolbar) {
        close_windows(app);
        return Err(error.to_string());
    }

    let stop = Arc::new(AtomicBool::new(false));
    let worker = std::thread::spawn({
        let (app, stop) = (app.clone(), Arc::clone(&stop));
        move || {
            let result = run(&app, monitor_id, zone, &stop);
            if let Err(error) = &result {
                eprintln!("scroll capture stopped: {error}");
                emit(&app, Progress { height: 0, max_height: stitch::MAX_HEIGHT, state: State::Failed });
            }
            result
        }
    });
    *app.state::<ScrollCapture>().0.lock().unwrap() = Some(Session { stop, worker, zone_width: zone.width });
    Ok(())
}

fn open_windows(app: &AppHandle, zone: Rect, toolbar: Rect) -> tauri::Result<()> {
    let (left, top) = (zone.x - EDGE as i32, zone.y - EDGE as i32);
    let (right, bottom) = (zone.x + zone.width as i32, zone.y + zone.height as i32);
    let edges = [
        Rect::new(left, top, zone.width + EDGE * 2, EDGE),
        Rect::new(left, bottom, zone.width + EDGE * 2, EDGE),
        Rect::new(left, zone.y, EDGE, zone.height),
        Rect::new(right, zone.y, EDGE, zone.height),
    ];
    for (index, edge) in edges.into_iter().enumerate() {
        let window = builder(app, &format!("{WINDOW_PREFIX}edge-{index}"), edge)
            .focused(false)
            .focusable(false)
            .background_color(EDGE_COLOR)
            .build()?;
        window.set_ignore_cursor_events(true)?;
        selector::raise_above_menu_bar(app, window);
    }
    let window = builder(app, TOOLBAR_WINDOW, toolbar).focused(true).accept_first_mouse(true).build()?;
    selector::raise_above_menu_bar(app, window);
    Ok(())
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
}

fn close_windows(app: &AppHandle) {
    for (label, window) in app.webview_windows() {
        if label.starts_with(WINDOW_PREFIX) {
            let _ = window.destroy();
        }
    }
}

fn emit(app: &AppHandle, progress: Progress) {
    if let Err(error) = app.emit_to(TOOLBAR_WINDOW, PROGRESS_EVENT, progress) {
        eprintln!("scroll-progress emit failed: {error}");
    }
}

/// The capture loop. Sampling keeps a steady pace without piling up: a slow capture just
/// delays the next one.
fn run(app: &AppHandle, monitor_id: u32, zone: Rect, stop: &AtomicBool) -> Result<Stitcher, String> {
    std::thread::sleep(SETTLE);
    let monitors = xcap::Monitor::all().map_err(|error| error.to_string())?;
    let monitor = monitors
        .into_iter()
        .find(|monitor| monitor.id().is_ok_and(|id| id == monitor_id))
        .ok_or("the monitor is gone")?;
    let (x, y) = (zone.x.max(0) as u32, zone.y.max(0) as u32);
    let grab = || monitor.capture_region(x, y, zone.width, zone.height).map_err(|error| error.to_string());

    let mut stitcher = Stitcher::new(&grab()?);
    let mut state = State::Capturing;
    emit(app, Progress { height: stitcher.height(), max_height: stitch::MAX_HEIGHT, state: state.clone() });
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
        emit(app, Progress { height: stitcher.height(), max_height: stitch::MAX_HEIGHT, state: state.clone() });
        std::thread::sleep(SAMPLE_INTERVAL.saturating_sub(started.elapsed()));
    }
    Ok(stitcher)
}

fn take_session(app: &AppHandle) -> Option<Session> {
    let session = app.state::<ScrollCapture>().0.lock().unwrap().take();
    if let Some(session) = &session {
        session.stop.store(true, Ordering::SeqCst);
        close_windows(app);
    }
    session
}

/// Discards the capture. The worker notices the stop flag and ends by itself.
fn abandon(app: &AppHandle) {
    if take_session(app).is_some() {
        capture::selection_ended(app);
    }
}

/// A frame or toolbar window closed some other way must not leave the others behind.
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
