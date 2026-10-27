import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import type { Notify } from "./ui-helpers";

/**
 * Opening images that were not captured here. Rust reads the pasteboard and does the
 * converting; a result arrives as a normal `capture-ready`, a problem as `editor-notice`.
 */
export function openClipboard(notify: Notify): void {
  invoke("open_clipboard_image").catch((error) => notify(`No se pudo abrir el portapapeles: ${error}`, true));
}

/** The system open dialog, shown by Rust; PNG and JPEG only. */
export function openFile(notify: Notify): void {
  invoke("open_image_file").catch((error) => notify(`No se pudo abrir el archivo: ${error}`, true));
}

/**
 * Highlights `target` while files are dragged over the window. Rust opens what is dropped,
 * with the paths the system gives it; the page never sees or sends them.
 */
export function highlightDrops(target: HTMLElement): void {
  getCurrentWebview()
    .onDragDropEvent((event) => {
      const { type } = event.payload;
      target.classList.toggle("drop-target", type === "enter" || type === "over");
    })
    .catch((error) => console.error("drag and drop events unavailable", error));
}

/** Messages from actions started outside the editor, such as the menu bar icon. */
export function listenNotices(notify: Notify): void {
  listen<{ message: string; error: boolean }>("editor-notice", (event) => {
    notify(event.payload.message, event.payload.error);
  }).catch((error) => console.error("editor notices unavailable", error));
}
