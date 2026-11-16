//! Windows only: WebView2 enables browser-style accelerator keys (F5/Ctrl+R reload the page,
//! Ctrl+P prints, Ctrl+F searches…) by default, which macOS's WKWebView never does. Left on,
//! F5 would silently discard whatever capture is being edited — called once per window, right
//! after it is created, for every window in the app (editor, settings, selector overlays,
//! scroll frame/toolbar, HUD bezel). Zoom (Ctrl+wheel/Ctrl+Plus/Minus) needs no equivalent
//! fix: `zoom_hotkeys_enabled` already defaults to `false` in wry, which disables WebView2's
//! own zoom control on Windows the same way.

use tauri::WebviewWindow;
use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Settings3;
use windows::core::Interface;

pub fn disable_browser_accelerator_keys(window: &WebviewWindow) {
    let label = window.label().to_string();
    let reached = window.with_webview(move |webview| {
        // SAFETY: `controller()`/`CoreWebView2()`/`Settings()` are plain COM property getters
        // on a live WebView2 controller, which Tauri hands out for its own window.
        let result = (|| -> windows::core::Result<()> {
            let core = unsafe { webview.controller().CoreWebView2()? };
            let settings = unsafe { core.Settings()? };
            if let Ok(settings3) = settings.cast::<ICoreWebView2Settings3>() {
                unsafe { settings3.SetAreBrowserAcceleratorKeysEnabled(false)? };
            }
            Ok(())
        })();
        if let Err(error) = result {
            eprintln!("browser accelerator keys could not be disabled for window {label}: {error}");
        }
    });
    if let Err(error) = reached {
        eprintln!("could not reach the webview of window {} to disable accelerator keys: {error}", window.label());
    }
}
