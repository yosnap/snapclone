/** Whether the app runs on macOS. WebView2 on Windows always reports "Windows NT" here, so
 * this needs no round-trip to Rust. */
export const IS_MAC = navigator.userAgent.includes("Mac");

/** Windows only: any other non-Mac system (Linux) keeps the macOS-style custom-protocol URLs. */
export const IS_WINDOWS = navigator.userAgent.includes("Windows");

/**
 * Where Rust's `capture://` custom protocol (registered in `lib.rs`) is actually reachable
 * from the webview. WKWebView (macOS) routes the scheme itself, `capture://localhost/...`, to
 * the registered handler — but WebView2 (Windows) does not support arbitrary custom schemes
 * that way: Tauri instead serves them as `http://capture.localhost/...` there (the same
 * platform split its own Rust-side `convert_file_src`/JS `convertFileSrc` document). Using the
 * macOS form on Windows makes the webview treat it as an unknown scheme, so the request never
 * reaches the handler and `<img>.decode()` fails with "the source image cannot be decoded".
 */
export const CAPTURE_ORIGIN = IS_WINDOWS ? "http://capture.localhost" : "capture://localhost";

/** The app's "primary" shortcut modifier: ⌘ on macOS, Ctrl on Windows. */
export function isPrimaryModifier(event: KeyboardEvent | MouseEvent): boolean {
  return IS_MAC ? event.metaKey : event.ctrlKey;
}

/**
 * The OTHER platform's primary modifier, which this app never uses as a shortcut modifier:
 * Ctrl on macOS, Win on Windows (`event.metaKey` there). Exposed for conditions that mean
 * "no modifier besides the primary one" — substituting `isPrimaryModifier` alone into such a
 * condition without also replacing its `event.ctrlKey` check would leave that check always
 * true on Windows (Ctrl already being the primary modifier there), making the shortcut
 * impossible to press.
 */
export function isSecondaryModifier(event: KeyboardEvent | MouseEvent): boolean {
  return IS_MAC ? event.ctrlKey : event.metaKey;
}

/** How a shortcut is written in menus/tooltips: `⇧⌘S` on macOS, `Ctrl+Shift+S` on Windows. */
export function formatShortcut(key: string, mods: { shift?: boolean; alt?: boolean } = {}): string {
  if (IS_MAC) {
    return `${mods.alt ? "⌥" : ""}${mods.shift ? "⇧" : ""}⌘${key}`;
  }
  return `Ctrl+${mods.alt ? "Alt+" : ""}${mods.shift ? "Shift+" : ""}${key}`;
}

/**
 * Adds `platform-mac`/`platform-windows` to `<body>`, the single source every page uses to
 * tell the two apart in CSS. Call once, as early as possible in each page's entry script.
 */
export function applyPlatformClass(body: HTMLElement = document.body): void {
  body.classList.add(IS_MAC ? "platform-mac" : IS_WINDOWS ? "platform-windows" : "platform-other");
}
