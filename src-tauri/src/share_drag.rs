//! "Compartir" and "Arrástrame": the exported image, written by `output::prepare_export`,
//! handed to the macOS share menu or dragged out of the editor as a PNG file. Both commands
//! are synchronous, so Tauri runs them on the main thread, as AppKit requires.

use tauri::{AppHandle, Manager};

use crate::{capture::EDITOR_WINDOW, output};

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

fn editor_ns_window(app: &AppHandle) -> Result<usize, String> {
    let window = app.get_webview_window(EDITOR_WINDOW).ok_or("the editor window is gone")?;
    window.ns_window().map(|pointer| pointer as usize).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn share_export(app: AppHandle, anchor: Anchor) -> Result<(), String> {
    if !anchor.is_valid() {
        return Err("invalid share button position".into());
    }
    let path = output::export_file(&app)?;
    macos::share(editor_ns_window(&app)?, &path, anchor)
}

#[tauri::command]
pub fn drag_export(app: AppHandle) -> Result<(), String> {
    let path = output::export_file(&app)?;
    macos::drag(editor_ns_window(&app)?, &path)
}

#[cfg(target_os = "macos")]
mod macos {
    use std::{cell::RefCell, path::Path};

    use objc2::{
        AnyThread, MainThreadMarker, MainThreadOnly, define_class, msg_send,
        rc::Retained,
        runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject},
    };
    use objc2_app_kit::{
        NSApplication, NSDragOperation, NSDraggingContext, NSDraggingItem, NSDraggingSession, NSDraggingSource,
        NSEventType, NSImage, NSSharingServicePicker, NSView, NSWindow,
    };
    use objc2_foundation::{NSArray, NSPoint, NSRect, NSRectEdge, NSSize, NSString, NSURL};

    use super::Anchor;

    /// Side of the image shown under the pointer while dragging, in points.
    const DRAG_PREVIEW_SIDE: f64 = 160.0;

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

    fn content_view(ns_window: usize) -> Result<(MainThreadMarker, Retained<NSView>), String> {
        let mtm = MainThreadMarker::new().ok_or("AppKit must be called on the main thread")?;
        // SAFETY: Tauri returns the editor's live NSWindow; it outlives this call on the main thread.
        let window = unsafe { &*(ns_window as *const NSWindow) };
        Ok((mtm, window.contentView().ok_or("the editor has no content view")?))
    }

    fn file_url(path: &Path) -> Result<Retained<NSURL>, String> {
        let text = path.to_str().ok_or("the export path is not valid UTF-8")?;
        Ok(NSURL::fileURLWithPath(&NSString::from_str(text)))
    }

    pub fn share(ns_window: usize, path: &Path, anchor: Anchor) -> Result<(), String> {
        let (_mtm, view) = content_view(ns_window)?;
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

    pub fn drag(ns_window: usize, path: &Path) -> Result<(), String> {
        let (mtm, view) = content_view(ns_window)?;
        // The page asks while the mouse is being dragged over it, so the event AppKit is
        // handling is that drag; a session can only start from a mouse event.
        let event = NSApplication::sharedApplication(mtm).currentEvent().ok_or("there is no mouse event to drag from")?;
        if !matches!(event.r#type(), NSEventType::LeftMouseDragged | NSEventType::LeftMouseDown) {
            return Err("the mouse button is no longer pressed".into());
        }
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
        Ok(())
    }
}

#[cfg(not(target_os = "macos"))]
mod macos {
    use std::path::Path;

    pub fn share(_ns_window: usize, _path: &Path, _anchor: super::Anchor) -> Result<(), String> {
        Err("sharing is only implemented on macOS".into())
    }

    pub fn drag(_ns_window: usize, _path: &Path) -> Result<(), String> {
        Err("dragging out is only implemented on macOS".into())
    }
}
