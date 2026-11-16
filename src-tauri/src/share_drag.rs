//! "Compartir" and "Arrástrame": the exported image, written by `output::prepare_export`,
//! handed to the macOS share menu or dragged out of the editor as a PNG file. Both commands
//! are synchronous, so Tauri runs them on the main thread, as AppKit requires.

use tauri::AppHandle;

use crate::output;

/// Where the share button is, in CSS pixels from the top-left of the editor's page.
#[derive(serde::Deserialize, Clone, Copy)]
pub struct Anchor {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl Anchor {
    fn is_valid(self) -> bool {
        [self.x, self.y, self.width, self.height].iter().all(|value| value.is_finite() && value.abs() < 1e5)
            && self.width >= 0.0
            && self.height >= 0.0
    }
}

// macOS and the generic fallback: SYNCHRONOUS, exactly as before Phase 4 (zero change here).
#[cfg(not(target_os = "windows"))]
#[tauri::command]
pub fn share_export(app: AppHandle, anchor: Anchor) -> Result<(), String> {
    if !anchor.is_valid() {
        return Err("invalid share button position".into());
    }
    let path = output::export_file(&app)?;
    #[cfg(target_os = "macos")]
    { macos::share(&app, &path, anchor) }
    #[cfg(not(target_os = "macos"))]
    { other::share(&app, &path, anchor) }
}

/// Windows is `async` on purpose (red team): `windows::share` resolves a `StorageFile` with
/// `.join()`, forbidden on the main thread (see Phase 3), so that part runs in `spawn_blocking`
/// and only the window-bound half (`GetForWindow`/`DataRequested`/`ShowShareUIForWindow`) jumps
/// to the main thread via `run_on_main_thread`. Never make this synchronous again.
#[cfg(target_os = "windows")]
#[tauri::command]
pub async fn share_export(app: AppHandle, anchor: Anchor) -> Result<(), String> {
    if !anchor.is_valid() {
        return Err("invalid share button position".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let path = output::export_file(&app)?;
        // `anchor` has no Windows equivalent (`IDataTransferManagerInterop` only takes a
        // `HWND`, no anchor rectangle); kept in the signature for parity with the other
        // platforms, simply unused here.
        windows::share(&app, &path, anchor)
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Starts dragging the prepared file. Returns false, without an error, when the mouse button
/// was already released: the press simply ended before the drag began.
///
/// SYNCHRONOUS on every platform, on purpose (opposite rule from `share_export` above): on
/// Windows, `drag::init_ole()` pins OLE to the first thread that ever starts a drag, which must
/// always be this same thread — making this `async` (or wrapping it in `run_on_main_thread`,
/// which would deadlock since this already runs there) would break that guarantee.
#[tauri::command]
pub fn drag_export(app: AppHandle) -> Result<bool, String> {
    let path = output::export_file(&app)?;
    #[cfg(target_os = "macos")]
    { macos::drag(&app, &path) }
    #[cfg(target_os = "windows")]
    { windows::drag(&app, &path) }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    { other::drag(&app, &path) }
}

#[cfg(target_os = "macos")]
mod macos {
    use std::{cell::RefCell, path::Path};

    use tauri::{AppHandle, Manager};

    use objc2::{
        AnyThread, MainThreadMarker, MainThreadOnly, define_class, msg_send,
        rc::Retained,
        runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject},
    };
    use objc2_app_kit::{
        NSApplication, NSDragOperation, NSDraggingContext, NSDraggingItem, NSDraggingSession, NSDraggingSource, NSEvent,
        NSEventModifierFlags, NSEventType, NSImage, NSSharingServicePicker, NSView, NSWindow,
    };
    use objc2_foundation::{NSArray, NSPoint, NSProcessInfo, NSRect, NSRectEdge, NSSize, NSString, NSURL};

    use crate::capture::EDITOR_WINDOW;

    use super::Anchor;

    /// Side of the image shown under the pointer while dragging, in points.
    const DRAG_PREVIEW_SIDE: f64 = 160.0;

    fn editor_ns_window(app: &AppHandle) -> Result<usize, String> {
        let window = app.get_webview_window(EDITOR_WINDOW).ok_or("the editor window is gone")?;
        window.ns_window().map(|pointer| pointer as usize).map_err(|error| error.to_string())
    }

    define_class!(
        // SAFETY: NSObject has no subclassing requirements and this class has no Drop.
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "SnapcloneDragSource"]
        struct DragSource;

        unsafe impl NSObjectProtocol for DragSource {}

        unsafe impl NSDraggingSource for DragSource {
            /// The file is offered as a copy, so the editor's image is never moved away.
            #[unsafe(method(draggingSession:sourceOperationMaskForDraggingContext:))]
            fn source_operation_mask(&self, _session: &NSDraggingSession, _context: NSDraggingContext) -> NSDragOperation {
                NSDragOperation::Copy
            }
        }
    );

    impl DragSource {
        fn new(mtm: MainThreadMarker) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(());
            // SAFETY: NSObject's designated initialiser.
            unsafe { msg_send![super(this), init] }
        }
    }

    thread_local! {
        /// AppKit does not retain the dragging source, and the share picker must outlive its
        /// menu, so the latest of each is kept on the main thread.
        static DRAG_SOURCE: RefCell<Option<Retained<DragSource>>> = const { RefCell::new(None) };
        static PICKER: RefCell<Option<Retained<NSSharingServicePicker>>> = const { RefCell::new(None) };
    }

    fn editor_window<'a>(ns_window: usize) -> Result<(MainThreadMarker, &'a NSWindow, Retained<NSView>), String> {
        let mtm = MainThreadMarker::new().ok_or("AppKit must be called on the main thread")?;
        // SAFETY: Tauri returns the editor's live NSWindow; it outlives this call on the main thread.
        let window = unsafe { &*(ns_window as *const NSWindow) };
        let view = window.contentView().ok_or("the editor has no content view")?;
        Ok((mtm, window, view))
    }

    /// The left button is still held down (bit 0 of the pressed buttons).
    fn left_button_pressed() -> bool {
        NSEvent::pressedMouseButtons() & 1 != 0
    }

    /**
     * The mouse event to start the session from. The one AppKit is handling when the page's
     * request arrives is used if it is the drag itself; otherwise, with the button still
     * down, an equivalent drag event is built at the pointer's current position, so the
     * session never depends on what else AppKit happened to be processing.
     */
    fn drag_event(mtm: MainThreadMarker, window: &NSWindow) -> Option<Retained<NSEvent>> {
        let current = NSApplication::sharedApplication(mtm)
            .currentEvent()
            .filter(|event| matches!(event.r#type(), NSEventType::LeftMouseDragged | NSEventType::LeftMouseDown));
        if current.is_some() {
            return current;
        }
        NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
            NSEventType::LeftMouseDragged,
            window.mouseLocationOutsideOfEventStream(),
            NSEventModifierFlags::empty(),
            NSProcessInfo::processInfo().systemUptime(),
            window.windowNumber(),
            None,
            0,
            1,
            1.0,
        )
    }

    fn file_url(path: &Path) -> Result<Retained<NSURL>, String> {
        let text = path.to_str().ok_or("the export path is not valid UTF-8")?;
        Ok(NSURL::fileURLWithPath(&NSString::from_str(text)))
    }

    pub fn share(app: &AppHandle, path: &Path, anchor: Anchor) -> Result<(), String> {
        let (_mtm, _window, view) = editor_window(editor_ns_window(app)?)?;
        let url = file_url(path)?;
        let item: &AnyObject = &url;
        let items = NSArray::from_slice(&[item]);
        // SAFETY: the array holds an NSURL, which the picker accepts.
        let picker = unsafe { NSSharingServicePicker::initWithItems(NSSharingServicePicker::alloc(), &items) };
        let height = view.bounds().size.height;
        // The page measures from the top; an unflipped view measures from the bottom.
        let y = if view.isFlipped() { anchor.y } else { height - anchor.y - anchor.height };
        let rect = NSRect::new(NSPoint::new(anchor.x, y), NSSize::new(anchor.width, anchor.height));
        picker.showRelativeToRect_ofView_preferredEdge(rect, &view, NSRectEdge::MinY);
        PICKER.with(|cell| *cell.borrow_mut() = Some(picker));
        Ok(())
    }

    pub fn drag(app: &AppHandle, path: &Path) -> Result<bool, String> {
        let (mtm, window, view) = editor_window(editor_ns_window(app)?)?;
        if !left_button_pressed() {
            return Ok(false);
        }
        let event = drag_event(mtm, window).ok_or("no se pudo crear el evento de arrastre")?;
        let url = file_url(path)?;
        let item = NSDraggingItem::initWithPasteboardWriter(NSDraggingItem::alloc(), ProtocolObject::from_ref(&*url));
        let location = view.convertPoint_fromView(event.locationInWindow(), None);
        let image = NSImage::initWithContentsOfFile(NSImage::alloc(), &NSString::from_str(path.to_str().unwrap_or_default()));
        let size = image.as_ref().map(|image| image.size()).filter(|size| size.width > 0.0 && size.height > 0.0);
        let factor = size.map_or(1.0, |size| DRAG_PREVIEW_SIDE / size.width.max(size.height));
        let (width, height) = size.map_or((DRAG_PREVIEW_SIDE, DRAG_PREVIEW_SIDE), |size| (size.width * factor, size.height * factor));
        let frame = NSRect::new(NSPoint::new(location.x - width / 2.0, location.y - height / 2.0), NSSize::new(width, height));
        let contents: Option<&AnyObject> = image.as_deref().map(|image| image as &AnyObject);
        // SAFETY: the contents are an NSImage, as the dragging frame expects.
        unsafe { item.setDraggingFrame_contents(frame, contents) };
        let source = DRAG_SOURCE.with(|cell| cell.borrow_mut().get_or_insert_with(|| DragSource::new(mtm)).clone());
        let items = NSArray::from_retained_slice(&[item]);
        view.beginDraggingSessionWithItems_event_source(&items, &event, ProtocolObject::from_ref(&*source));
        Ok(true)
    }
}

#[cfg(target_os = "windows")]
mod windows {
    use std::{
        cell::RefCell,
        panic::{AssertUnwindSafe, catch_unwind},
        path::Path,
        sync::{Arc, Mutex, mpsc},
        time::Duration,
    };

    use drag::{DragItem, DragResult, Image, Options};
    use tauri::{AppHandle, Manager};
    use windows::{
        ApplicationModel::DataTransfer::{DataRequestedEventArgs, DataTransferManager},
        Foundation::TypedEventHandler,
        Storage::{IStorageItem, StorageFile},
        Win32::{
            Foundation::HWND,
            UI::{
                Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON, VK_RBUTTON},
                Shell::IDataTransferManagerInterop,
                WindowsAndMessaging::{GetForegroundWindow, GetSystemMetrics, SM_SWAPBUTTON},
            },
        },
        core::{AgileReference, Error, HSTRING, Interface, Ref, factory},
    };
    use windows_collections::IIterable;

    use super::Anchor;

    /// Side of the image shown under the pointer while dragging, matching macOS's
    /// `DRAG_PREVIEW_SIDE`, so a full-size scroll capture is never decoded just for a preview.
    const DRAG_PREVIEW_SIDE: u32 = 160;

    thread_local! {
        // `GetForWindow` always returns the SAME manager for a given `HWND`: without this, a
        // handler would be registered again on every "Compartir" click and they would pile up.
        static ACTIVE_SHARE: RefCell<Option<(DataTransferManager, i64)>> = const { RefCell::new(None) };
    }

    fn win_msg(context: &str, error: Error) -> String {
        format!("{context}: {} (0x{:08X})", error.message(), error.code().0 as u32)
    }

    /// Carries the resolved file across to `run_on_main_thread`: `AgileReference` and `HSTRING`
    /// are `Send`, the `StorageFile`/`IStorageItem` they stand in for are not.
    struct PreparedShare {
        item: AgileReference<IStorageItem>,
        title: HSTRING,
    }

    /// Call ONLY from a worker (`spawn_blocking`), never the main thread: this does `.join()`
    /// (forbidden there, see Phase 3) and then waits on the main thread for the panel itself.
    pub fn share(app: &AppHandle, path: &Path, _anchor: Anchor) -> Result<(), String> {
        let prepared = prepare_share(path)?;
        let (sender, receiver) = mpsc::channel();
        let main_app = app.clone();
        app.run_on_main_thread(move || {
            let _ = sender.send(show_share(&main_app, prepared));
        })
        .map_err(|error| error.to_string())?;
        receiver.recv_timeout(Duration::from_secs(5)).map_err(|_| "el panel de compartir no respondió".to_string())?
    }

    /// Blocking half: `.join()` is safe here, this never runs on the main thread.
    fn prepare_share(path: &Path) -> Result<PreparedShare, String> {
        let file: StorageFile = StorageFile::GetFileFromPathAsync(&HSTRING::from(path))
            .and_then(|operation| operation.join())
            .map_err(|error| win_msg("no se pudo abrir la imagen exportada", error))?;
        let item = file
            .cast::<IStorageItem>()
            .and_then(|item| AgileReference::new(&item))
            .map_err(|error| win_msg("no se pudo preparar la imagen exportada", error))?;
        let title = HSTRING::from(path.file_name().unwrap_or(path.as_os_str()));
        Ok(PreparedShare { item, title })
    }

    /// Main-thread half: only ever called from inside `run_on_main_thread`.
    fn show_share(app: &AppHandle, prepared: PreparedShare) -> Result<(), String> {
        let PreparedShare { item, title } = prepared;
        let window = app.get_webview_window(crate::capture::EDITOR_WINDOW).ok_or("the editor window is gone")?;
        let hwnd: HWND = window.hwnd().map_err(|error| error.to_string())?;
        // SAFETY: `hwnd` is the editor's live window, read on the main thread as required.
        unsafe {
            // Checked BEFORE registering anything (red team): failing after would leave an
            // orphaned handler armed.
            if GetForegroundWindow() != hwnd {
                return Err("la ventana del editor no está en primer plano".into());
            }
            let interop: IDataTransferManagerInterop = factory::<DataTransferManager, IDataTransferManagerInterop>()
                .map_err(|error| win_msg("el panel de compartir de Windows no está disponible", error))?;
            let manager: DataTransferManager = interop
                .GetForWindow(hwnd)
                .map_err(|error| win_msg("el panel de compartir de Windows no está disponible", error))?;
            ACTIVE_SHARE.with_borrow_mut(|active| {
                if let Some((old, token)) = active.take() {
                    let _ = old.RemoveDataRequested(token);
                }
            });
            let handler = TypedEventHandler::<DataTransferManager, DataRequestedEventArgs>::new(move |_, args| {
                // NEVER return `Err` here: that crosses WinRT's `extern` boundary and Windows
                // may kill the whole process for it (confirmed against Chrome's own source, see
                // the Windows port plan). `fill_share_request`'s own `catch_unwind` keeps a panic
                // from doing the same thing — but ONLY in a dev build: this crate's release
                // profile sets `panic = "abort"` (Cargo.toml), which aborts the process before
                // unwinding even starts, so `catch_unwind` never runs there. It is a real
                // diagnostic safety net for dev, not a release-time guarantee.
                fill_share_request(&args, &item, &title);
                Ok(())
            });
            let token =
                manager.DataRequested(&handler).map_err(|error| win_msg("no se pudo preparar el panel de compartir", error))?;
            ACTIVE_SHARE.with_borrow_mut(|active| *active = Some((manager, token)));
            interop.ShowShareUIForWindow(hwnd).map_err(|error| win_msg("no se pudo abrir el panel de compartir", error))
        }
    }

    fn fill_share_request(args: &Ref<DataRequestedEventArgs>, item: &AgileReference<IStorageItem>, title: &HSTRING) {
        let Some(request) = args.as_ref().and_then(|args| args.Request().ok()) else { return };
        let outcome = catch_unwind(AssertUnwindSafe(|| -> windows::core::Result<()> {
            let data = request.Data()?;
            data.Properties()?.SetTitle(title)?;
            data.SetStorageItemsReadOnly(&IIterable::<IStorageItem>::from(vec![Some(item.resolve()?)]))
        }));
        // `Err(panic)` or `Ok(Err(hresult))`: same treatment, never propagated further.
        if !matches!(outcome, Ok(Ok(()))) {
            let _ = request.FailWithDisplayText(&HSTRING::from("No se pudo preparar la imagen para compartir."));
        }
    }

    /// The PRIMARY (logical "select") mouse button is still held down, same intent as macOS's
    /// `left_button_pressed()`. `GetAsyncKeyState` reports the PHYSICAL button regardless of the
    /// user's "switch primary and secondary buttons" setting (confirmed against Microsoft's own
    /// docs for `VK_LBUTTON`), so for a left-handed user whose buttons are swapped, checking
    /// `VK_LBUTTON` alone would watch the wrong (now-secondary) physical button.
    fn left_button_pressed() -> bool {
        // SAFETY: plain state queries, no resources involved.
        unsafe {
            let primary = if GetSystemMetrics(SM_SWAPBUTTON) != 0 { VK_RBUTTON } else { VK_LBUTTON };
            GetAsyncKeyState(i32::from(primary.0)) < 0
        }
    }

    /// A PNG thumbnail no larger than `DRAG_PREVIEW_SIDE` on its longest side: `drag` decodes
    /// `Image::Raw` on the main thread before the drag starts, so a full scroll-capture-sized
    /// PNG would stall the UI and show an oversized ghost image.
    fn preview_image(path: &Path) -> Result<Image, String> {
        // `xcap::image::open` decodes under the crate's default 512MiB `max_alloc`, which a
        // scroll capture's own export can exceed — the same reason `export.rs::decode_rgba`
        // reads this file's bytes through `decode_limits()` instead of a convenience opener.
        let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
        let mut reader =
            xcap::image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().map_err(|error| error.to_string())?;
        reader.limits(crate::import::decode_limits());
        let image = reader.decode().map_err(|error| error.to_string())?;
        let longest = image.width().max(image.height());
        // `.thumbnail()` (fast integer box filter) rather than `.resize(..., Lanczos3)`: this
        // whole function runs on the main thread before the drag starts, and Lanczos3 on a
        // scroll-capture-sized image is slow enough to visibly stall the UI for a mere preview.
        let thumbnail = if longest > DRAG_PREVIEW_SIDE { image.thumbnail(DRAG_PREVIEW_SIDE, DRAG_PREVIEW_SIDE) } else { image };
        let mut png = Vec::new();
        thumbnail
            .write_to(&mut std::io::Cursor::new(&mut png), xcap::image::ImageFormat::Png)
            .map_err(|error| error.to_string())?;
        Ok(Image::Raw(png))
    }

    // Must keep being called from a SYNCHRONOUS command (main thread): `drag::init_ole` pins
    // OLE to the first thread that ever drags. Never make this `async` or wrap it in
    // `run_on_main_thread` — asymmetric to `share` above on purpose, see `drag_export`.
    pub fn drag(app: &AppHandle, path: &Path) -> Result<bool, String> {
        if !left_button_pressed() {
            return Ok(false); // same as macOS: the press ended before the drag began
        }
        let window = app.get_webview_window(crate::capture::EDITOR_WINDOW).ok_or("the editor window is gone")?;
        let preview = preview_image(path)?;
        // Decoding/thumbnailing above can take a moment on a large capture: re-check that the
        // button is still down, in case the press already ended while that was running — same
        // reasoning as the check at the top of this function, just after the one slow step.
        if !left_button_pressed() {
            return Ok(false);
        }
        // The callback must be `Fn(...) + Send + 'static`: neither `Cell` nor `Rc` are `Send`,
        // so the outcome is shared through an `Arc<Mutex<_>>` cloned before the closure.
        let outcome = Arc::new(Mutex::new(DragResult::Cancel));
        let callback_outcome = Arc::clone(&outcome);
        drag::start_drag(
            &window,
            DragItem::Files(vec![path.to_path_buf()]),
            preview,
            move |result, _cursor_position| *callback_outcome.lock().unwrap() = result,
            Options::default(),
        )
        .map_err(|error| error.to_string())?;
        // `start_drag` blocks on Windows: by the time it returns, the drag has already ended.
        let result = *outcome.lock().unwrap();
        Ok(matches!(result, DragResult::Dropped))
    }
}

/// Placeholder for every platform that is neither macOS nor Windows (same `share`/
/// `drag(app: &AppHandle, ...)` contract as the real `mod windows` below).
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod other {
    use std::path::Path;

    use tauri::AppHandle;

    pub fn share(_app: &AppHandle, _path: &Path, _anchor: super::Anchor) -> Result<(), String> {
        Err("sharing is only implemented on macOS".into())
    }

    pub fn drag(_app: &AppHandle, _path: &Path) -> Result<bool, String> {
        Err("dragging out is only implemented on macOS".into())
    }
}
