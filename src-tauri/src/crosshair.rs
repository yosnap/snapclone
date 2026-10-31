//! The crosshair over the selector overlays. WebKit applies the page's CSS cursor only in
//! the key window, and a borderless overlay never becomes key, so the pointer kept the
//! arrow. A tracking area that is always active sets the crosshair itself on every move,
//! whichever monitor the pointer is on.

use tauri::WebviewWindow;

#[cfg(target_os = "macos")]
mod native {
    use std::cell::OnceCell;

    use objc2::{
        define_class, msg_send,
        rc::Retained,
        runtime::{NSObject, NSObjectProtocol},
        AllocAnyThread, MainThreadMarker, MainThreadOnly,
    };
    use objc2_app_kit::{NSCursor, NSEvent, NSTrackingArea, NSTrackingAreaOptions, NSWindow};
    use objc2_foundation::NSRect;

    define_class!(
        // SAFETY: NSObject has no subclassing requirements and this class has no Drop.
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "SnapcloneCrosshairTracker"]
        pub struct Tracker;

        unsafe impl NSObjectProtocol for Tracker {}

        impl Tracker {
            #[unsafe(method(cursorUpdate:))]
            fn cursor_update(&self, _event: &NSEvent) {
                NSCursor::crosshairCursor().set();
            }

            #[unsafe(method(mouseEntered:))]
            fn mouse_entered(&self, _event: &NSEvent) {
                NSCursor::crosshairCursor().set();
            }

            #[unsafe(method(mouseMoved:))]
            fn mouse_moved(&self, _event: &NSEvent) {
                NSCursor::crosshairCursor().set();
            }

            // Required, not optional: while the app is inactive, as during a selection,
            // AppKit sends the exit of an always-active area without checking that the
            // owner responds, and the unrecognised selector aborts the process.
            #[unsafe(method(mouseExited:))]
            fn mouse_exited(&self, _event: &NSEvent) {}
        }
    );

    thread_local! {
        /// A tracking area does not retain its owner, so one tracker lives for the whole
        /// process and serves every overlay.
        static TRACKER: OnceCell<Retained<Tracker>> = const { OnceCell::new() };
    }

    fn tracker(mtm: MainThreadMarker) -> Retained<Tracker> {
        TRACKER.with(|cell| {
            cell.get_or_init(|| {
                let this = Tracker::alloc(mtm).set_ivars(());
                // SAFETY: NSObject's designated initialiser.
                unsafe { msg_send![super(this), init] }
            })
            .clone()
        })
    }

    /// Must run on the main thread with the overlay's live NSWindow.
    pub fn attach(ns_window: &NSWindow, mtm: MainThreadMarker) {
        let Some(view) = ns_window.contentView() else { return };
        let options = NSTrackingAreaOptions::CursorUpdate
            | NSTrackingAreaOptions::MouseEnteredAndExited
            | NSTrackingAreaOptions::MouseMoved
            | NSTrackingAreaOptions::ActiveAlways
            | NSTrackingAreaOptions::InVisibleRect;
        let owner = tracker(mtm);
        // SAFETY: the owner outlives the area (it is never released), and AppKit is only
        // touched on the main thread.
        let area = unsafe {
            NSTrackingArea::initWithRect_options_owner_userInfo(
                NSTrackingArea::alloc(),
                NSRect::ZERO,
                options,
                Some(&owner),
                None,
            )
        };
        view.addTrackingArea(&area);
        ns_window.setAcceptsMouseMovedEvents(true);
        NSCursor::crosshairCursor().set();
    }
}

/// Shows the crosshair over an overlay from now on, without moving the pointer first.
pub fn track(window: &WebviewWindow) {
    #[cfg(target_os = "macos")]
    {
        let overlay = window.clone();
        let result = window.run_on_main_thread(move || {
            let Some(mtm) = objc2::MainThreadMarker::new() else { return };
            let Ok(pointer) = overlay.ns_window() else { return };
            // SAFETY: Tauri hands out the live NSWindow of this window, and AppKit is only
            // touched here on the main thread.
            let ns_window = unsafe { &*pointer.cast::<objc2_app_kit::NSWindow>() };
            native::attach(ns_window, mtm);
        });
        if let Err(error) = result {
            eprintln!("the crosshair could not be set up: {error}");
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = window;
}
